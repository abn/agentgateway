import { useState, useEffect, forwardRef, useImperativeHandle } from "react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from "@/components/ui/collapsible";
import { ChevronUp, ChevronDown } from "lucide-react";
import { Header, TargetWithType, StreamableHttpTarget } from "@/lib/types"; // Added StreamableHttpTarget

interface StreamableHttpTargetFormProps {
  targetName: string;
  onSubmit: (target: TargetWithType) => Promise<void>;
  isLoading: boolean;
  existingTarget?: TargetWithType;
  hideSubmitButton?: boolean;
}

export const StreamableHttpTargetForm = forwardRef<
  { submitForm: () => Promise<void> },
  StreamableHttpTargetFormProps
>(({ targetName, onSubmit, isLoading, existingTarget, hideSubmitButton = false }, ref) => {
  const [targetUrl, setTargetUrl] = useState(""); // Renamed from sseUrl to targetUrl
  const [showAdvancedSettings, setShowAdvancedSettings] = useState(false);
  const [headers, setHeaders] = useState<Header[]>([]);
  const [headerKey, setHeaderKey] = useState("");
  const [headerValue, setHeaderValue] = useState("");
  const [passthroughAuth, setPassthroughAuth] = useState(false);
  const [insecureSkipVerify, setInsecureSkipVerify] = useState(false);
  const [selectedListeners, setSelectedListeners] = useState<string[]>([]);

  // Initialize form with existing target data if provided
  useEffect(() => {
    if (existingTarget) {
      if (existingTarget.streamable_http) { // Changed from existingTarget.sse
        const targetData = existingTarget.streamable_http; // Changed from sse
        const protocol = targetData.tls?.insecure_skip_verify ? "https" : "http";
        const url = `${protocol}://${targetData.host}:${targetData.port}${targetData.path}`;
        setTargetUrl(url); // Changed from setSseUrl

        if (targetData.headers) {
          setHeaders(targetData.headers);
        }

        if (targetData.auth?.passthrough) {
          setPassthroughAuth(true);
        }

        if (targetData.tls?.insecure_skip_verify) {
          setInsecureSkipVerify(true);
        }
      }
      if (existingTarget.listeners) {
        setSelectedListeners(existingTarget.listeners);
      }
    }
  }, [existingTarget]);

  const addHeader = () => {
    if (headerKey && headerValue) {
      setHeaders([...headers, { key: headerKey, value: { string_value: headerValue } }]);
      setHeaderKey("");
      setHeaderValue("");
    }
  };

  const removeHeader = (index: number) => {
    setHeaders(headers.filter((_, i) => i !== index));
  };

  const handleSubmit = async () => {
    try {
      const urlObj = new URL(targetUrl); // Changed from sseUrl
      let port: number;
      if (urlObj.port) {
        port = parseInt(urlObj.port, 10);
      } else {
        port = urlObj.protocol === "https:" ? 443 : 80;
      }

      const targetData: StreamableHttpTarget = { // Explicitly type as StreamableHttpTarget
        host: urlObj.hostname,
        port: port,
        path: urlObj.pathname + urlObj.search,
        headers: headers.length > 0 ? headers : undefined,
      };

      const target: TargetWithType = {
        name: targetName,
        type: "streamable_http", // Changed type to "streamable_http"
        listeners: selectedListeners,
        streamable_http: targetData, // Changed from sse to streamable_http
      };

      // Add auth if passthrough is enabled
      if (passthroughAuth) {
        target.streamable_http!.auth = { // Changed from sse to streamable_http
          passthrough: true,
        };
      }

      // Add TLS config if insecure skip verify is enabled
      if (insecureSkipVerify) {
        target.streamable_http!.tls = { // Changed from sse to streamable_http
          insecure_skip_verify: true,
        };
      }

      await onSubmit(target as TargetWithType);
    } catch (err) {
      console.error("Error creating Streamable HTTP target:", err); // Updated error message
      throw err;
    }
  };

  useImperativeHandle(ref, () => ({
    submitForm: handleSubmit,
  }));

  return (
    <form
      id="mcp-target-form" // Consider changing id if needed, though not critical
      onSubmit={(e) => {
        e.preventDefault();
        handleSubmit();
      }}
      className="space-y-4 pt-4"
    >
      <div className="space-y-2">
        <Label htmlFor="targetUrl">Server URL</Label> {/* Changed from sseUrl */}
        <Input
          id="targetUrl" // Changed from sseUrl
          type="url"
          value={targetUrl} // Changed from sseUrl
          onChange={(e) => setTargetUrl(e.target.value)} // Changed from setSseUrl
          placeholder="http://localhost:8080/mcp" // Updated placeholder
          required
        />
        <p className="text-sm text-muted-foreground">
          Enter the full URL for the Streamable HTTP MCP endpoint.
        </p>
      </div>

      <Collapsible open={showAdvancedSettings} onOpenChange={setShowAdvancedSettings}>
        <CollapsibleTrigger asChild>
          <Button variant="ghost" className="flex items-center p-0 h-auto">
            {showAdvancedSettings ? (
              <ChevronUp className="h-4 w-4 mr-1" />
            ) : (
              <ChevronDown className="h-4 w-4 mr-1" />
            )}
            Advanced Settings
          </Button>
        </CollapsibleTrigger>
        <CollapsibleContent className="space-y-4 pt-2">
          <div className="space-y-4">
            <div className="space-y-2">
              <Label>Headers</Label>
              <div className="space-y-2">
                {headers.map((header, index) => (
                  <div key={index} className="flex items-center gap-2">
                    <div className="flex-1">
                      <Input value={header.key} disabled placeholder="Header key" />
                    </div>
                    <div className="flex-1">
                      <Input
                        value={header.value.string_value}
                        disabled
                        placeholder="Header value"
                      />
                    </div>
                    <Button
                      type="button"
                      variant="outline"
                      size="icon"
                      onClick={() => removeHeader(index)}
                    >
                      <span className="sr-only">Remove header</span>
                      <svg
                        xmlns="http://www.w3.org/2000/svg"
                        width="24"
                        height="24"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke="currentColor"
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                        className="h-4 w-4"
                      >
                        <path d="M18 6 6 18" />
                        <path d="m6 6 12 12" />
                      </svg>
                    </Button>
                  </div>
                ))}
                <div className="flex items-center gap-2">
                  <div className="flex-1">
                    <Input
                      value={headerKey}
                      onChange={(e) => setHeaderKey(e.target.value)}
                      placeholder="Header key"
                    />
                  </div>
                  <div className="flex-1">
                    <Input
                      value={headerValue}
                      onChange={(e) => setHeaderValue(e.target.value)}
                      placeholder="Header value"
                    />
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    onClick={addHeader}
                    disabled={!headerKey || !headerValue}
                  >
                    Add
                  </Button>
                </div>
              </div>
            </div>

            <div className="space-y-2">
              <Label>Authentication</Label>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="passthrough-auth-streamable" // Changed id for uniqueness
                  checked={passthroughAuth}
                  onCheckedChange={(checked: boolean | "indeterminate") =>
                    setPassthroughAuth(checked as boolean)
                  }
                />
                <Label htmlFor="passthrough-auth-streamable" className="text-sm font-normal"> {/* Changed htmlFor */}
                  Pass through authentication
                </Label>
              </div>
            </div>

            <div className="space-y-2">
              <Label>TLS Configuration</Label>
              <div className="flex items-center space-x-2">
                <Checkbox
                  id="insecure-skip-verify-streamable" // Changed id for uniqueness
                  checked={insecureSkipVerify}
                  onCheckedChange={(checked: boolean | "indeterminate") =>
                    setInsecureSkipVerify(checked as boolean)
                  }
                />
                <Label htmlFor="insecure-skip-verify-streamable" className="text-sm font-normal"> {/* Changed htmlFor */}
                  Insecure skip verify
                </Label>
              </div>
            </div>
          </div>
        </CollapsibleContent>
      </Collapsible>

      {!hideSubmitButton && (
        <Button
          type="submit"
          className="w-full"
          disabled={isLoading || !targetUrl || selectedListeners.length === 0} // Changed from !sseUrl
        >
          {isLoading
            ? existingTarget
              ? "Updating Target..."
              : "Creating Target..."
            : existingTarget
              ? "Update Target"
              : "Create Target"}
        </Button>
      )}
    </form>
  );
});

StreamableHttpTargetForm.displayName = "StreamableHttpTargetForm";
