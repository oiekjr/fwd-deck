import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { setImmediate } from "node:timers/promises";
import test from "node:test";
import ts from "typescript";

const source = await readFile(new URL("../src/lib/command-dispatcher.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 },
});
const { createCommandDispatcher } = await import(
  `data:text/javascript;base64,${Buffer.from(outputText).toString("base64")}`
);

/** 完了タイミングをテスト側から制御する */
function deferred() {
  return Promise.withResolvers();
}

/** 画面とネイティブ操作が共有する受付キューを模擬する */
function createBackend() {
  let tail = Promise.resolve();

  /** 受け付け済みの操作が完了してから次の処理を実行する */
  return function submit(action) {
    const result = tail.then(action);
    tail = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  };
}

test("応答待ちの画面起動要求を後続のネイティブ Workspace 切り替えが追い越さない", async () => {
  const version = deferred();
  const submit = createBackend();
  let workspace = "A";
  const running = new Set();
  const dispatch = createCommandDispatcher((command) =>
    submit(async () => {
      if (command === "load_version_status") {
        await version.promise;
      } else {
        workspace = "A";
        running.add("A:db");
      }
    }),
  );

  const versionResponse = dispatch("load_version_status", {});
  const startResponse = dispatch("start_tunnels", {});
  const workspaceSwitch = submit(() => {
    running.delete("A:db");
    workspace = "B";
  });
  version.resolve();
  await Promise.all([versionResponse, startResponse, workspaceSwitch]);

  assert.equal(workspace, "B");
  assert.equal(running.size, 0);
});

test("先に要求した画面起動が終了時の停止対象に含まれる", async () => {
  const version = deferred();
  const submit = createBackend();
  const running = new Set(["existing"]);
  let quitTargets = [];
  const dispatch = createCommandDispatcher((command) =>
    submit(async () => {
      if (command === "load_version_status") {
        await version.promise;
      } else {
        running.add("new");
      }
    }),
  );

  const versionResponse = dispatch("load_version_status", {});
  const startResponse = dispatch("start_tunnels", {});
  const quit = submit(() => {
    quitTargets = [...running];
  });
  version.resolve();
  await Promise.all([versionResponse, startResponse, quit]);

  assert.deepEqual(quitTargets, ["existing", "new"]);
});

test("送信は即座に行い、逆順に届いた応答を要求順に返す", async () => {
  const first = deferred();
  const second = deferred();
  const sent = [];
  const received = [];
  const dispatch = createCommandDispatcher((command) => {
    sent.push(command);
    return command === "first" ? first.promise : second.promise;
  });

  const firstResponse = dispatch("first", {}).then((value) => received.push(value));
  const secondResponse = dispatch("second", {}).then((value) => received.push(value));
  second.resolve("second");
  await setImmediate();

  assert.deepEqual(sent, ["first", "second"]);
  assert.deepEqual(received, []);

  first.resolve("first");
  await Promise.all([firstResponse, secondResponse]);

  assert.deepEqual(received, ["first", "second"]);
});

test("先行応答の待機中に届いたエラーを未処理の rejection にせず伝える", async () => {
  const first = deferred();
  const failure = new Error("second failed");
  const dispatch = createCommandDispatcher((command) =>
    command === "first" ? first.promise : Promise.reject(failure),
  );

  const firstResponse = dispatch("first", {});
  const secondResponse = assert.rejects(dispatch("second", {}), (error) => error === failure);
  await setImmediate();
  first.resolve("first");

  assert.equal(await firstResponse, "first");
  await secondResponse;
});

test("先行要求の失敗後も後続の成功応答を返す", async () => {
  const failure = new Error("first failed");
  const dispatch = createCommandDispatcher((command) =>
    command === "first" ? Promise.reject(failure) : Promise.resolve("second"),
  );

  const firstResponse = assert.rejects(dispatch("first", {}), (error) => error === failure);
  const secondResponse = dispatch("second", {});

  await firstResponse;
  assert.equal(await secondResponse, "second");
});
