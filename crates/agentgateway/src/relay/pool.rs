use crate::outbound;
// Import for the protobuf type Target_OpenAPITarget
use crate::proto::agentgateway::dev::mcp::target::target::OpenApiTarget as ProtoXdsOpenApiTarget;

use super::*;
use futures::StreamExt; // Removed BoxFuture, FutureExt
use reqwest::{Client as HttpClient, IntoUrl, Url, header::ACCEPT};
use rmcp::service::{Peer, serve_client_with_ct, NotificationContext, RequestContext};
use rmcp::transport::sse_client::{SseClient, SseTransportError, SseClientConfig};
use rmcp::transport::common::client_side_sse::FixedInterval;
use rmcp::transport::SseClientTransport;
use rmcp::{ClientHandler, ServiceError, Error as McpError, RoleClient, RoleServer};
use rmcp::model::{
    ClientRequest, ClientResult, ClientNotification, JsonRpcMessage,
    CreateMessageRequestParam, CreateMessageResult, ListRootsResult,
    CancelledNotificationParam, ProgressNotificationParam, LoggingMessageNotificationParam,
    ResourceUpdatedNotificationParam,
    // For OpenAPI part of connect method:
    InitializeResult, ListToolsResult, CallToolRequestParam, CallToolResult, Content, RequestId,
    // ServerResult, // Removed based on warnings/usage check
    // PaginatedRequestParam, // Removed based on warnings/usage check
    // Request as ModelRequest, // Removed based on warnings
    // Extensions, GetExtensions, GetMeta, ServerInfo, ClientInfo, // Removed based on warnings
    // InitializeRequest, InitializeRequestParam, // Removed based on warnings (InitializeResult is kept)
    // ListToolsRequest, ListPromptsRequest, ListResourcesRequest, ListResourceTemplatesRequest, // Removed
    // ReadResourceRequest, ReadResourceRequestParam, ReadResourceResult // Removed
};
use sse_stream::{Error as SseError, Sse, SseStream};
use http::Uri;
// use std::pin::Pin; // Unused
use futures::stream::BoxStream; // Keep BoxStream as it's used, remove Stream
use futures::Future;
use std::sync::Arc;
use tokio::process::Command;
use http::{HeaderMap, HeaderName, HeaderValue, header::AUTHORIZATION}; // For ReqwestSseClient if headers were added back


pub(crate) struct ConnectionPool {
	listener_name: String,
	state: Arc<tokio::sync::RwLock<XdsStore>>,
	by_name: HashMap<String, upstream::UpstreamTarget>,
}

impl ConnectionPool {
	pub(crate) fn new(state: Arc<tokio::sync::RwLock<XdsStore>>, listener_name: String) -> Self {
		Self {
			listener_name,
			state,
			by_name: HashMap::new(),
		}
	}

	pub(crate) async fn get_or_create(
		&mut self,
		rq_ctx: &RqCtx,
		peer: &Peer<RoleServer>,
		name: &str,
	) -> anyhow::Result<&upstream::UpstreamTarget> {
		if !self.by_name.contains_key(name) {
			let target_info: Option<(
				outbound::Target<outbound::McpTargetSpec>,
				tokio_util::sync::CancellationToken,
			)> = {
				let state = self.state.read().await;
				state
					.mcp_targets
					.get(name, &self.listener_name)
					.map(|(target, ct)| (target.clone(), ct.clone()))
			};

			if let Some((target, ct)) = target_info {
				self.connect(rq_ctx, &ct, &target, peer).await?;
			} else {
				return Err(anyhow::anyhow!(
					"Target configuration not found for {}",
					name
				));
			}
		}
		let target = self.by_name.get(name);
		Ok(target.ok_or(McpError::invalid_request(
			format!("Service {} not found", name),
			None,
		))?)
	}

	pub(crate) async fn remove(&mut self, name: &str) -> Option<upstream::UpstreamTarget> {
		self.by_name.remove(name)
	}

	pub(crate) async fn list(
		&mut self,
		rq_ctx: &RqCtx,
		peer: &Peer<RoleServer>,
	) -> anyhow::Result<Vec<(String, &upstream::UpstreamTarget)>> {
		let targets_config: Vec<(
			String,
			(
				outbound::Target<outbound::McpTargetSpec>,
				tokio_util::sync::CancellationToken,
			),
		)> = {
			let state = self.state.read().await;
			state
				.mcp_targets
				.iter(&self.listener_name)
				.map(|(name, target)| (name.clone(), target.clone()))
				.collect()
		};

		let mut connections_to_make = Vec::new();
		for (name, (target, ct)) in &targets_config {
			if !self.by_name.contains_key(name) {
				connections_to_make.push((name.clone(), target.clone(), ct.clone()));
			}
		}

		for (name, target, ct) in connections_to_make {
			tracing::debug!("Connecting missing target: {}", name);
			self
				.connect(rq_ctx, &ct, &target, peer)
				.await
				.map_err(|e| {
					tracing::error!("Failed to connect target {}: {}", name, e);
					e
				})?;
		}
		tracing::debug!("Finished connecting missing targets.");

		let results = targets_config
			.into_iter()
			.filter_map(|(name, _)| {
				self
					.by_name
					.get(&name)
					.map(|target: &upstream::UpstreamTarget| (name, target))
			})
			.collect();

		Ok(results)
	}

	#[instrument(
    level = "debug",
    skip_all,
    fields(
        name=%target.name,
    ),
  )]
	pub(crate) async fn connect(
		&mut self,
		rq_ctx: &RqCtx,
		ct: &tokio_util::sync::CancellationToken,
		target: &outbound::Target<outbound::McpTargetSpec>,
		peer: &Peer<RoleServer>,
	) -> Result<(), anyhow::Error> {
		if let Some(_transport) = self.by_name.get(&target.name) {
			return Ok(());
		}
		tracing::trace!("connecting to target: {}", target.name);
		let transport_to_insert: upstream::UpstreamTarget = match &target.spec {
			McpTargetSpec::Sse(sse) => {
				tracing::debug!("starting sse transport for target: {}", target.name);
				let path = match sse.path.as_str() {
					"" => "/sse",
					_ => sse.path.as_str(),
				};
				let builder = reqwest::Client::builder();
				let (scheme, builder) = tls_cfg(builder, &sse.tls, sse.port).await?;

				let url_str = format!("{}://{}:{}{}", scheme, sse.host, sse.port, path);
				let mut upstream_headers = get_default_headers(&sse.backend_auth, rq_ctx).await?;
				for (key, value) in sse.headers.iter() {
					upstream_headers.insert(
						HeaderName::from_bytes(key.as_bytes())?,
						HeaderValue::from_str(value)?,
					);
				}
				let http_client = builder.default_headers(upstream_headers).build()?;
				let sse_client_for_transport = ReqwestSseClient::new_with_client(url_str.as_str(), http_client).await?;

                let sse_client_config = SseClientConfig {
                    sse_endpoint: Arc::from(url_str.as_str()),
                    retry_policy: Arc::new(FixedInterval::default()),
                    use_message_endpoint: None,
                };
				let transport_to_serve = SseClientTransport::start_with_client(sse_client_for_transport, sse_client_config).await?;

				upstream::UpstreamTarget {
					filters: target.filters.clone(),
					spec: upstream::UpstreamTargetSpec::Mcp(
						serve_client_with_ct(
							PeerClientHandler {
								peer: peer.clone(),
							},
							transport_to_serve,
							ct.child_token(),
						)
						.await?,
					),
				}
			},
			McpTargetSpec::Stdio { cmd, args, env: _ } => {
				tracing::debug!("starting stdio transport for target: {}", target.name);
				upstream::UpstreamTarget {
					filters: target.filters.clone(),
					spec: upstream::UpstreamTargetSpec::Mcp(
						serve_client_with_ct(
							PeerClientHandler {
								peer: peer.clone(),
							}, {
								let mut command = Command::new(cmd);
								command.args(args);
								TokioChildProcess::new(command)?
							},
							ct.child_token(),
						)
						.await?,
					),
				}
			},
			McpTargetSpec::OpenAPI(openapi_target_spec_from_outbound) => {
				tracing::debug!("starting OpenAPI transport for target: {}", target.name);
				let current_schema_source_proto = openapi_target_spec_from_outbound
					.schema_source
					.clone()
					.ok_or_else(|| {
					anyhow::anyhow!(
						"OpenAPI target {} is missing schema_source definition",
						target.name
					)
				})?;
				let proto_target_for_loading = ProtoXdsOpenApiTarget {
					host: openapi_target_spec_from_outbound.host.clone(),
					port: openapi_target_spec_from_outbound.port,
					schema_source: Some(current_schema_source_proto),
					auth: None,
					tls: None,
					headers: vec![],
				};
				let loaded_openapi_doc =
					crate::outbound::openapi::load_openapi_schema(&proto_target_for_loading)
						.await
						.map_err(|e| {
							anyhow::anyhow!(
								"Failed to load OpenAPI schema for target {}: {}",
								target.name,
								e
							)
						})?;
				let tools =
					crate::outbound::openapi::parse_openapi_schema(&loaded_openapi_doc).map_err(|e| {
						anyhow::anyhow!(
							"Failed to parse tools from OpenAPI schema for target {}: {}",
							target.name,
							e
						)
					})?;
				let server_info =
					crate::outbound::openapi::get_server_info(&loaded_openapi_doc).map_err(|e| {
						anyhow::anyhow!(
							"Failed to get server info from OpenAPI schema for target {}: {}",
							target.name,
							e
						)
					})?;
				let (final_scheme, final_host, final_port, final_prefix, builder) =
					if server_info.scheme.is_some() {
						let host = server_info.host.unwrap();
						let port = server_info.port;
						let builder = reqwest::Client::builder();
						let (verified_scheme, configured_builder) =
							tls_cfg(builder, &openapi_target_spec_from_outbound.tls, port).await?;
						(
							verified_scheme,
							host,
							port,
							server_info.path_prefix,
							configured_builder,
						)
					} else {
						let builder = reqwest::Client::builder();
						let (scheme, configured_builder) = tls_cfg(
							builder,
							&openapi_target_spec_from_outbound.tls,
							openapi_target_spec_from_outbound.port,
						)
						.await?;
						(
							scheme,
							openapi_target_spec_from_outbound.host.clone(),
							openapi_target_spec_from_outbound.port,
							server_info.path_prefix,
							configured_builder,
						)
					};
				let mut api_headers =
					get_default_headers(&openapi_target_spec_from_outbound.backend_auth, rq_ctx).await?;
				for (key, value) in &openapi_target_spec_from_outbound.headers {
					api_headers.insert(
						HeaderName::from_bytes(key.as_bytes())?,
						HeaderValue::from_str(value)?,
					);
				}
				let final_client = builder.default_headers(api_headers).build()?;
				upstream::UpstreamTarget {
					filters: target.filters.clone(),
					spec: upstream::UpstreamTargetSpec::OpenAPI(crate::outbound::openapi::Handler {
						host: final_host,
						client: final_client,
						tools,
						scheme: final_scheme,
						prefix: final_prefix,
						port: final_port,
					}),
				}
			},
		};
		self.by_name.insert(target.name.clone(), transport_to_insert);
		Ok(())
	}
}

async fn tls_cfg(
	builder: reqwest::ClientBuilder,
	tls: &Option<outbound::TlsConfig>,
	port: u32,
) -> Result<(String, reqwest::ClientBuilder), anyhow::Error> {
	match (port, tls) {
		(443, None) => {
			let builder = builder.https_only(true);
			Ok(("https".to_string(), builder))
		},
		(443, Some(tls_config)) => {
			let builder = builder
				.https_only(true)
				.danger_accept_invalid_hostnames(tls_config.insecure_skip_verify);
			Ok(("https".to_string(), builder))
		},
		(_, None) => Ok(("http".to_string(), builder)),
		(_, Some(tls_config)) => {
			let builder = builder
				.https_only(false)
				.danger_accept_invalid_hostnames(tls_config.insecure_skip_verify);
			Ok(("https".to_string(), builder))
		},
	}
}

#[derive(Debug, Clone)]
pub(crate) struct PeerClientHandler {
	peer: Peer<RoleServer>,
	// peer_client: Option<Peer<RoleClient>>, // Unused after removing set_peer/get_peer
}

impl ClientHandler for PeerClientHandler {
	async fn create_message(
		&self,
		params: CreateMessageRequestParam,
		_context: RequestContext<RoleClient>,
	) -> Result<CreateMessageResult, McpError> {
		self.peer.create_message(params).await.map_err(|e| match e {
			ServiceError::McpError(mcp_error) => mcp_error,
			_ => McpError::internal_error(e.to_string(), None),
		})
	}

	async fn list_roots(
		&self,
		_context: RequestContext<RoleClient>,
	) -> Result<ListRootsResult, McpError> {
		self.peer.list_roots().await.map_err(|e| match e {
			ServiceError::McpError(mcp_error) => mcp_error,
			_ => McpError::internal_error(e.to_string(), None),
		})
	}

	async fn on_cancelled(&self, params: CancelledNotificationParam, _context: NotificationContext<RoleClient>) {
		let _ = self.peer.notify_cancelled(params).await.inspect_err(|e| {
			tracing::error!("Failed to notify cancelled: {}", e);
		});
	}

	async fn on_progress(&self, params: ProgressNotificationParam, _context: NotificationContext<RoleClient>) {
		let _ = self.peer.notify_progress(params).await.inspect_err(|e| {
			tracing::error!("Failed to notify progress: {}", e);
		});
	}

	async fn on_logging_message(&self, params: LoggingMessageNotificationParam, _context: NotificationContext<RoleClient>) {
		let _ = self
			.peer
			.notify_logging_message(params)
			.await
			.inspect_err(|e| {
				tracing::error!("Failed to notify logging message: {}", e);
			});
	}

	async fn on_prompt_list_changed(&self, _context: NotificationContext<RoleClient>) {
		let _ = self
			.peer
			.notify_prompt_list_changed()
			.await
			.inspect_err(|e| {
				tracing::error!("Failed to notify prompt list changed: {}", e);
			});
	}

	async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
		let _ = self
			.peer
			.notify_resource_list_changed()
			.await
			.inspect_err(|e| {
				tracing::error!("Failed to notify resource list changed: {}", e);
			});
	}

	async fn on_tool_list_changed(&self, _context: NotificationContext<RoleClient>) {
		let _ = self.peer.notify_tool_list_changed().await.inspect_err(|e| {
			tracing::error!("Failed to notify tool list changed: {}", e);
		});
	}

	async fn on_resource_updated(&self, params: ResourceUpdatedNotificationParam, _context: NotificationContext<RoleClient>) {
		let _ = self
			.peer
			.notify_resource_updated(params)
			.await
			.inspect_err(|e| {
				tracing::error!("Failed to notify resource updated: {}", e);
			});
	}
}

async fn get_default_headers(
	auth_config: &Option<backend::BackendAuthConfig>,
	rq_ctx: &RqCtx,
) -> Result<HeaderMap, anyhow::Error> {
	match auth_config {
		Some(auth_config_val) => {
			let backend_auth = auth_config_val.build(&rq_ctx.identity).await?;
			let token = backend_auth.get_token().await?;
			let mut upstream_headers = HeaderMap::new();
			let auth_value = HeaderValue::from_str(&format!("Bearer {}", token))?;
			upstream_headers.insert(AUTHORIZATION, auth_value);
			Ok(upstream_headers)
		},
		None => Ok(HeaderMap::new()),
	}
}

#[derive(Clone)]
pub struct ReqwestSseClient {
	http_client: HttpClient,
	sse_url: Url,
}

impl ReqwestSseClient {
	pub async fn new_with_client<U>(
		url: U,
		client: HttpClient,
	) -> Result<Self, SseTransportError<reqwest::Error>>
	where
		U: IntoUrl,
	{
		let url = url.into_url()?;
		Ok(Self {
			http_client: client,
			sse_url: url,
		})
	}
}

// Constants for SseClient implementation
const MIME_TYPE_EVENT_STREAM: &str = "text/event-stream";
const MIME_TYPE_JSON: &str = "application/json";

impl SseClient for ReqwestSseClient {
    type Error = reqwest::Error;

    fn get_stream(
        &self,
        _uri: Uri,
        last_event_id: Option<String>,
        _auth_token: Option<String>,
    ) -> impl Future<Output = Result<BoxStream<'static, Result<Sse, SseError>>, SseTransportError<Self::Error>>> + Send + '_ {
        let client = self.http_client.clone();
        let sse_url_str = self.sse_url.to_string();

        async move {
            let mut request_builder = client.get(sse_url_str)
                .header(ACCEPT, MIME_TYPE_EVENT_STREAM);

            if let Some(leid) = last_event_id {
                request_builder = request_builder.header("Last-Event-ID", leid);
            }

            let response = request_builder.send().await?;
            let response = response.error_for_status()?;

            match response.headers().get(reqwest::header::CONTENT_TYPE) {
                Some(ct) => {
                    if !ct.as_bytes().starts_with(MIME_TYPE_EVENT_STREAM.as_bytes()) {
                        return Err(SseTransportError::UnexpectedContentType(Some(ct.clone())));
                    }
                }
                None => {
                    return Err(SseTransportError::UnexpectedContentType(None));
                }
            }
            let event_stream = SseStream::from_byte_stream(response.bytes_stream()).boxed();
            Ok(event_stream)
        }
    }

    fn post_message(
        &self,
        _uri: Uri,
        message: JsonRpcMessage<ClientRequest, ClientResult, ClientNotification>,
        _auth_token: Option<String>,
    ) -> impl Future<Output = Result<(), SseTransportError<Self::Error>>> + Send + '_ {
        let client = self.http_client.clone();
        let mcp_endpoint_url = self.sse_url.clone();

        async move {
            // RqCtx and custom header logic temporarily removed.
            // A HeaderMap can be created here if other default headers are needed.
            // let mut headers = HeaderMap::new();

            let request_builder = client.post(mcp_endpoint_url)
                .header(ACCEPT, format!("{}, {}", MIME_TYPE_JSON, MIME_TYPE_EVENT_STREAM))
                .json(&message);
                // .headers(headers); // Removed if headers is not used

            request_builder
                .send()
                .await?
                .error_for_status()?
                .bytes()
                .await?;
            Ok(())
        }
    }
}
