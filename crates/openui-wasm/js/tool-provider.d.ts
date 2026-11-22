
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

export interface ResponseDraw {
    /** Owned by the caller. */
    readonly evaluation: EvaluationOutput;
    readonly name: string | undefined;
    readonly context: unknown;
    readonly attributes: Record<string, string>;
    readonly error: string | undefined;
}

export interface RuntimeSession {
    /** Supplied options replace the callbacks for future deliveries, including calls in flight. A segment already being delivered keeps its callbacks. */
    setToolProvider(provider?: ToolProvider | null, options?: ToolProviderOptions): void;
    /** Draw the entire response so far. */
    setResponse(text: string | null, streaming: boolean): ResponseDraw;
}
