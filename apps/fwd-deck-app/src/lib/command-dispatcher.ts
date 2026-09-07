/** Tauri へのコマンド送信境界を表現する */
export interface CommandInvoker {
  <T>(command: string, args: Record<string, unknown>): Promise<T>;
}

/** 受信順を待つ間も成功と失敗の両方を保持する */
type CommandResponse<T> = { ok: true; value: T } | { ok: false; error: unknown };

/**
 * ネイティブ操作との受付順を保つため即座に送信し、画面への応答だけを順序化する
 */
export function createCommandDispatcher(invoke: CommandInvoker): CommandInvoker {
  let responseQueue: Promise<void> = Promise.resolve();

  /** 先行応答を待つ間も送信とエラーの受信を進める */
  return function dispatchCommand<T>(command: string, args: Record<string, unknown>): Promise<T> {
    const response: Promise<CommandResponse<T>> = invoke<T>(command, args).then(
      (value: T): CommandResponse<T> => ({ ok: true, value }),
      (error: unknown): CommandResponse<T> => ({ ok: false, error }),
    );
    const orderedResponse: Promise<CommandResponse<T>> = responseQueue.then(() => response);
    responseQueue = orderedResponse.then(() => undefined);

    return orderedResponse.then((result: CommandResponse<T>): T => {
      if (!result.ok) {
        throw result.error;
      }

      return result.value;
    });
  };
}
