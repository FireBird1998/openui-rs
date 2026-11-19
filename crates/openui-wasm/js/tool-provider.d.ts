
export type ToolArguments = Record<string, unknown>;
export type ToolProvider =
    | Record<string, (args: ToolArguments) => unknown>
    | { callTool(request: { name: string; arguments: ToolArguments }): unknown };

export interface ToolProviderOptions {
    /** Fixed until detach. */
    minimumMs?: bigint;
    onChange?: () => void;
    /** Borrowed for this callback. */
    onAction?: (output: ActionOutput) => void;
    onError?: (error: Error) => void;
}

export interface RuntimeSession {
    /** Supplied options replace the callbacks, including for calls in flight. */
    setToolProvider(provider?: ToolProvider | null, options?: ToolProviderOptions): void;
}
