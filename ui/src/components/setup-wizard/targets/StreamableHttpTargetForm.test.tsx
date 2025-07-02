import { render, fireEvent, waitFor } from "@testing-library/react";
import "@testing-library/jest-dom";
import { StreamableHttpTargetForm } from "./StreamableHttpTargetForm";
import { TargetWithType, StreamableHttpTarget } from "@/lib/types";

// Mock the child components and external dependencies if necessary
// e.g., jest.mock("@/components/ui/input", () => (props: any) => <input {...props} />);

describe("StreamableHttpTargetForm", () => {
  const mockOnSubmit = jest.fn();
  const defaultProps = {
    targetName: "test-target",
    onSubmit: mockOnSubmit,
    isLoading: false,
  };

  beforeEach(() => {
    mockOnSubmit.mockClear();
  });

  test("renders correctly and allows input", () => {
    const { getByLabelText, getByPlaceholderText } = render(
      <StreamableHttpTargetForm {...defaultProps} />
    );

    const urlInput = getByLabelText("Server URL") as HTMLInputElement;
    expect(urlInput).toBeInTheDocument();
    fireEvent.change(urlInput, { target: { value: "http://localhost:8080/mcp" } });
    expect(urlInput.value).toBe("http://localhost:8080/mcp");

    // Check for advanced settings (optional)
    const advancedSettingsButton = getByText("Advanced Settings");
    expect(advancedSettingsButton).toBeInTheDocument();
  });

  test("submits correct data for Streamable HTTP target", async () => {
    const { getByLabelText, getByText } = render(
      <StreamableHttpTargetForm {...defaultProps} hideSubmitButton={false} />
    );

    const urlInput = getByLabelText("Server URL");
    fireEvent.change(urlInput, { target: { value: "https://secure.example.com:8443/mcp_endpoint" } });

    // In a real scenario, you might need to interact with listener selection if it's part of this form
    // For this example, we assume selectedListeners is handled or mocked appropriately if it affects submission enabling

    const submitButton = getByText("Create Target"); // Assuming default button text when not existingTarget
    fireEvent.click(submitButton);

    await waitFor(() => {
      expect(mockOnSubmit).toHaveBeenCalledTimes(1);
      const expectedTargetData: StreamableHttpTarget = {
        host: "secure.example.com",
        port: 8443,
        path: "/mcp_endpoint",
        headers: undefined, // Or mock header input if testing that
        auth: undefined,    // Or mock auth input
        tls: undefined,     // Or mock tls input
      };
      const expectedOutput: TargetWithType = {
        name: "test-target",
        type: "streamable_http",
        listeners: [], // Default or mocked
        streamable_http: expectedTargetData,
      };
      expect(mockOnSubmit).toHaveBeenCalledWith(expect.objectContaining(expectedOutput));
    });
  });

  test("initializes with existing target data", () => {
    const existingTarget: TargetWithType = {
      name: "existing-stream-target",
      type: "streamable_http",
      listeners: ["listener1"],
      streamable_http: {
        host: "oldhost.com",
        port: 1234,
        path: "/oldpath",
        headers: [{ key: "X-Old", value: { string_value: "OldValue" } }],
        auth: { passthrough: true },
        tls: { insecure_skip_verify: true },
      },
    };
    const { getByLabelText, getByDisplayValue, getByRole } = render(
      <StreamableHttpTargetForm {...defaultProps} existingTarget={existingTarget} />
    );

    expect((getByLabelText("Server URL") as HTMLInputElement).value).toBe("https://oldhost.com:1234/oldpath");

    // Check advanced settings values
    // To check headers, you'd need to open advanced settings first
    // fireEvent.click(getByText("Advanced Settings"));
    // expect(getByDisplayValue("X-Old")).toBeInTheDocument(); // This requires more specific selectors

    // Check checkboxes
    expect((getByRole("checkbox", { name: /Pass through authentication/i }) as HTMLInputElement).checked).toBe(true);
    expect((getByRole("checkbox", { name: /Insecure skip verify/i }) as HTMLInputElement).checked).toBe(true);
  });

  // Add more tests for header management, auth, tls options, validation etc.
});
