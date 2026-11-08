// Tests of target/yup.mjs against the native `yup`: every example builds to the same bytes, the
// program prints the same messages and exits with the same codes, and the library gives the
// diagnostics as data. Run `cargo run -p yup-bundle` first, then `node --test js/yup.test.mjs`.

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const bundle = join(root, "target", "yup.mjs");
assert.ok(existsSync(bundle), "target/yup.mjs is missing: run `cargo run -p yup-bundle` first");
const yup = await import(pathToFileURL(bundle).href);
const examples = join(root, "examples");

// The native `yup`, built once.
const built = spawnSync("cargo", ["build", "-q", "-p", "yup-cli"], { cwd: root, stdio: "inherit" });
assert.equal(built.status, 0, "the native yup builds");
const program = join(root, "target", "debug", process.platform === "win32" ? "yup.exe" : "yup");

/** Runs the native `yup`, with `stdin`, and gives its exit code and output. */
function native(args, stdin = "") {
  const run = spawnSync(program, args, { cwd: root, input: stdin });
  return { code: run.status, stdout: run.stdout, stderr: run.stderr.toString() };
}

/** Runs `node target/yup.mjs`, with `stdin`, and gives its exit code and output. */
function bundled(args, stdin = "") {
  const run = spawnSync(process.execPath, [bundle, ...args], { cwd: root, input: stdin });
  return { code: run.status, stdout: run.stdout, stderr: run.stderr.toString() };
}

test("every example builds to the same bytes as the native tool", async () => {
  const names = readdirSync(examples).filter((name) => name.endsWith(".yup"));
  assert.ok(names.length >= 7, "the examples are found");
  for (const name of names) {
    const source = readFileSync(join(examples, name), "utf8");
    const inputs = {};
    for (const file of await yup.inputFiles(source)) {
      inputs[file.name] = readFileSync(join(examples, file.name), "utf8");
    }
    const workbook = await yup.build(source, inputs);
    assert.deepEqual(workbook.warnings, [], name);
    const expected = native(["build", join("examples", name), "-o", "-"]);
    assert.equal(expected.code, 0, expected.stderr);
    assert.ok(Buffer.from(workbook.xlsx).equals(expected.stdout), `${name} builds the same bytes`);
  }
});

test("the program prints what the native tool prints, and exits the same way", () => {
  const wrong = "yup 0.2\nworkbook : Workbook\nworkbook = { sheets = [main] }\nmain : Sheet\n\
main = { name = 'Main' }\nx : Formula at main $B$2\nx = y + 1\n";
  const cases = [
    { args: ["check", "-"], stdin: wrong },
    { args: ["show", "ground", join("examples", "minimal.yup")] },
    { args: ["show", "nowhere", "-"] },
    { args: ["build", "-"] },
    { args: ["frobnicate"] },
    { args: [] },
    { args: ["--version"] },
  ];
  for (const { args, stdin } of cases) {
    const expected = native(args, stdin);
    const found = bundled(args, stdin);
    const what = args.join(" ") || "no arguments";
    assert.equal(found.code, expected.code, `${what}: the exit code`);
    assert.equal(found.stderr, expected.stderr, `${what}: the messages`);
    assert.ok(found.stdout.equals(expected.stdout), `${what}: the output`);
  }
  assert.match(bundled(["check", "-"], wrong).stderr, /^<stdin>:7:5: error: unknown name `y`/);
});

test("an input file that cannot be read is reported where the source names it", () => {
  const source = "yup 0.2\nworkbook : Workbook\nworkbook = { sheets = [main] }\nmain : Sheet\n\
main = { name = 'Main' }\nt : Value at main $B$2\nt = csv \"nowhere.csv\"\n";
  const found = bundled(["check", "-"], source);
  assert.equal(found.code, 1);
  assert.match(found.stderr, /^<stdin>:7:9: error: cannot read nowhere\.csv: /);
});

test("the library gives problems as data", async () => {
  const source = "yup 0.2\nx : Formula at main $B$2\nx = 1 +\n";
  await assert.rejects(yup.build(source), (error) => {
    assert.ok(error instanceof yup.YupError);
    assert.deepEqual(error.diagnostics, [
      {
        severity: "error",
        file: null,
        line: 3,
        column: 8,
        message: "expected a value, found the end of the line",
      },
    ]);
    assert.equal(error.message, "3:8: error: expected a value, found the end of the line");
    return true;
  });
  await assert.rejects(yup.show(source, "nowhere"), RangeError);
  assert.equal(
    yup.format({ severity: "warning", file: "t.csv", line: 2, column: 1, message: "m" }, "x.yup"),
    "t.csv:2:1: warning: m",
  );
  assert.equal(yup.steps.join(" "), "parse resolve infer place ground check lower package");
  assert.equal(yup.languageVersion, "0.2");
});

test("the library takes input files as a map, and shows a step", async () => {
  const source = readFileSync(join(examples, "history.yup"), "utf8");
  const history = readFileSync(join(examples, "history.csv"), "utf8");
  const shown = await yup.show(source, "ground", new Map([["history.csv", history]]));
  const expected = native(["show", "ground", join("examples", "history.yup")]);
  assert.equal(shown.text, expected.stdout.toString());
  const { warnings } = await yup.check(source, { "history.csv": history });
  assert.deepEqual(warnings, []);
});
