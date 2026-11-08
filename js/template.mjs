// yup.mjs: turns a .yup file and its CSV inputs into an xlsx workbook.
//
// The tool's core, compiled to WebAssembly, is embedded below, so this one file needs nothing
// but a JavaScript runtime with WebAssembly: Node 18 or later, Deno, Bun, or a browser.
//
//   As a program:  node yup.mjs build model.yup -o model.xlsx
//   As a library:  import { build } from "./yup.mjs"
//
// Licensed under the Apache License, Version 2.0.
//
// This is the template that the bundler, `cargo run -p yup-bundle`, fills in. Everything above
// the line that starts the command line uses no API of Node's, so that a page can embed it.

const WASM = "__YUP_WASM__";

/** The version of the tool. */
export const version = "__YUP_VERSION__";

/** The version of the `.yup` language the tool reads. */
export const languageVersion = "__YUP_LANGUAGE__";

/** The steps from a `.yup` file to a workbook, in order, as `show` takes them. */
export const steps = Object.freeze([
  "parse",
  "resolve",
  "infer",
  "place",
  "ground",
  "check",
  "lower",
  "package",
]);

/**
 * @typedef {object} Diagnostic
 * @property {"error" | "warning"} severity
 * @property {string | null} file The input file the problem is in, or null for the source.
 * @property {number} line Counted from 1.
 * @property {number} column Counted from 1, in characters.
 * @property {string} message
 */

/** Problems that stopped a step, each with where it is. */
export class YupError extends Error {
  /** @param {Diagnostic[]} diagnostics */
  constructor(diagnostics) {
    super(diagnostics.map((diagnostic) => format(diagnostic)).join("\n"));
    this.name = "YupError";
    /** @type {Diagnostic[]} */
    this.diagnostics = diagnostics;
  }
}

/**
 * A diagnostic as the command line prints it: `3:7: error: unknown name `x``, after the name of
 * the file it is in. A diagnostic in the source is given the source's name, if one is given.
 *
 * @param {Diagnostic} diagnostic
 * @param {string} [source]
 */
export function format(diagnostic, source) {
  const file = diagnostic.file ?? source;
  const where = file === undefined ? "" : `${file}:`;
  const { line, column, severity, message } = diagnostic;
  return `${where}${line}:${column}: ${severity}: ${message}`;
}

/**
 * The input files a source reads, in the order it names them, each with where.
 *
 * @param {string} source
 * @returns {Promise<{ name: string, line: number, column: number }[]>}
 */
export async function inputFiles(source) {
  const out = await call(0, 0, source, {});
  const files = [];
  for (let count = out.number(); count > 0; count--) {
    files.push({ name: out.text(), line: out.number(), column: out.number() });
  }
  return files;
}

/**
 * Runs the steps up to Check, and returns the warnings; throws a YupError for problems.
 *
 * @param {string} source
 * @param {Inputs} [inputs]
 */
export async function check(source, inputs = {}) {
  const { warnings } = await show(source, "check", inputs);
  return { warnings };
}

/**
 * Runs the steps up to `step`, and returns what it shows, with the warnings.
 *
 * @param {string} source
 * @param {string} step One of `steps`.
 * @param {Inputs} [inputs]
 */
export async function show(source, step, inputs = {}) {
  const index = steps.indexOf(step);
  if (index < 0) {
    throw new RangeError(`unknown step \`${step}\`; the steps are ${steps.join(", ")}`);
  }
  const out = await call(1, index, source, inputs);
  return { text: out.text(), warnings: out.diagnostics() };
}

/**
 * Runs every step and returns the workbook's bytes, with the warnings.
 *
 * @param {string} source
 * @param {Inputs} [inputs] The input files' contents, by the names the source gives them.
 */
export async function build(source, inputs = {}) {
  const out = await call(2, 0, source, inputs);
  return { xlsx: out.bytes(), warnings: out.diagnostics() };
}

/** @typedef {Record<string, string> | Map<string, string>} Inputs */

// The module, made once and kept. A bug in the core stops it with a trap, after which it is
// made again.
/** @type {Promise<any> | undefined} */
let loaded;

function core() {
  loaded ??= (async () => {
    const binary = atob(WASM);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) {
      bytes[i] = binary.charCodeAt(i);
    }
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return instance.exports;
  })();
  return loaded;
}

/**
 * Sends a request to the module and reads its response, as `crates/yup-wasm` lays both out.
 * Gives a reader positioned after the status for a response that is done; throws otherwise.
 *
 * @param {number} command 0 for the input files, 1 for show, 2 for build.
 * @param {number} step
 * @param {string} source
 * @param {Inputs} inputs
 */
async function call(command, step, source, inputs) {
  const exports = await core();
  const request = encode(command, step, source, inputs);
  let response;
  try {
    // Memory may grow during either call, so each view of it is taken afresh.
    const at = exports.request(request.length);
    new Uint8Array(exports.memory.buffer, at, request.length).set(request);
    const length = exports.run();
    response = new Uint8Array(exports.memory.buffer, exports.response(), length).slice();
  } catch (error) {
    loaded = undefined;
    throw new Error(`internal error in the core: ${error}`, { cause: error });
  }
  const out = new Reader(response);
  const status = out.byte();
  if (status === 1) {
    throw new YupError(out.diagnostics());
  }
  if (status !== 0) {
    throw new Error(`internal error: the core could not read its request: ${out.text()}`);
  }
  return out;
}

const encoder = new TextEncoder();

/**
 * @param {number} command
 * @param {number} step
 * @param {string} source
 * @param {Inputs} inputs
 */
function encode(command, step, source, inputs) {
  const entries = inputs instanceof Map ? [...inputs] : Object.entries(inputs);
  const texts = [source];
  for (const [name, contents] of entries) {
    if (typeof contents !== "string") {
      throw new TypeError(`the input file ${name} is not text`);
    }
    texts.push(name, contents);
  }
  const encoded = texts.map((text) => encoder.encode(text));
  const length = 2 + 4 + encoded.reduce((sum, bytes) => sum + 4 + bytes.length, 0);
  const request = new Uint8Array(length);
  const view = new DataView(request.buffer);
  request[0] = command;
  request[1] = step;
  let at = 2;
  const put = (/** @type {Uint8Array} */ bytes) => {
    view.setUint32(at, bytes.length, true);
    request.set(bytes, at + 4);
    at += 4 + bytes.length;
  };
  put(encoded[0]);
  view.setUint32(at, entries.length, true);
  at += 4;
  for (const bytes of encoded.slice(1)) {
    put(bytes);
  }
  return request;
}

const decoder = new TextDecoder("utf-8", { fatal: true });

/** Reads a response from its start. */
class Reader {
  /** @param {Uint8Array} data */
  constructor(data) {
    this.data = data;
    this.view = new DataView(data.buffer, data.byteOffset, data.byteLength);
    this.at = 0;
  }

  byte() {
    return this.data[this.at++];
  }

  number() {
    const value = this.view.getUint32(this.at, true);
    this.at += 4;
    return value;
  }

  bytes() {
    const length = this.number();
    const value = this.data.slice(this.at, this.at + length);
    this.at += length;
    return value;
  }

  text() {
    return decoder.decode(this.bytes());
  }

  /** @returns {Diagnostic[]} */
  diagnostics() {
    const diagnostics = [];
    for (let count = this.number(); count > 0; count--) {
      const severity = this.byte() === 0 ? "error" : "warning";
      const file = this.byte() === 1 ? this.text() : null;
      diagnostics.push({
        severity,
        file,
        line: this.number(),
        column: this.number(),
        message: this.text(),
      });
    }
    return diagnostics;
  }
}

// --- command line ---
// Everything below runs only when this file is run as a program, and is the only part that uses
// Node: it reads and writes files as `yup` does, with the same messages and exit codes.

const USAGE = `\
usage: yup build <file.yup> [-o <file.xlsx>]
       yup check <file.yup>
       yup show <step> <file.yup>

build   runs every step and writes the workbook, by default beside the .yup file
check   runs the steps up to Check and reports any problems
show    prints what one step makes of the file: parse, resolve, infer, place, ground
        and check print a .yup file, lower prints the cells, package the file's parts

A file named - is standard input, and -o - writes the workbook to standard output.`;

/** Why the command stopped early. */
class Failure {
  /**
   * @param {"usage" | "io" | "reported"} kind
   * @param {string} [message]
   */
  constructor(kind, message = "") {
    this.kind = kind;
    this.message = message;
  }
}

async function isMain() {
  if (typeof process === "undefined" || !process.argv?.[1]) {
    return false;
  }
  const { realpathSync } = await import("node:fs");
  const { fileURLToPath } = await import("node:url");
  try {
    return realpathSync(fileURLToPath(import.meta.url)) === realpathSync(process.argv[1]);
  } catch {
    return false;
  }
}

if (await isMain()) {
  process.exitCode = await main(process.argv.slice(2));
}

/** @param {string[]} args */
async function main(args) {
  try {
    await run(parseArgs(args));
    return 0;
  } catch (failure) {
    if (!(failure instanceof Failure)) {
      throw failure;
    }
    if (failure.kind === "usage") {
      process.stderr.write(`yup: ${failure.message}\n\n${USAGE}\n`);
      return 2;
    }
    if (failure.kind === "io") {
      process.stderr.write(`yup: ${failure.message}\n`);
    }
    return 1;
  }
}

/** @param {string[]} args */
function parseArgs(args) {
  const usage = (/** @type {string} */ message) => new Failure("usage", message);
  const [command, ...rest] = args;
  switch (command) {
    case undefined:
      throw usage("no command given");
    case "build": {
      let input;
      let output;
      for (let i = 0; i < rest.length; i++) {
        const word = rest[i];
        if (word === "-o" || word === "--output") {
          output = rest[++i];
          if (output === undefined) {
            throw usage("-o needs a file name");
          }
        } else if (input === undefined) {
          input = word;
        } else {
          throw usage(`unexpected argument \`${word}\``);
        }
      }
      if (input === undefined) {
        throw usage("build needs a .yup file");
      }
      return { command, input, output };
    }
    case "check":
      if (rest.length !== 1) {
        throw usage("check takes one .yup file");
      }
      return { command, input: rest[0] };
    case "show": {
      if (rest.length !== 2) {
        throw usage("show takes a step and a .yup file");
      }
      const [step, input] = rest;
      if (!steps.includes(step)) {
        throw usage(`unknown step \`${step}\`; the steps are ${steps.join(", ")}`);
      }
      return { command, step, input };
    }
    case "help":
    case "-h":
    case "--help":
      return { command: "help" };
    case "version":
    case "-V":
    case "--version":
      return { command: "version" };
    default:
      throw usage(`unknown command \`${command}\``);
  }
}

/** @param {any} command */
async function run(command) {
  switch (command.command) {
    case "help":
      process.stdout.write(`${USAGE}\n`);
      return;
    case "version":
      process.stdout.write(`yup ${version}, reading .yup ${languageVersion}\n`);
      return;
    case "build": {
      const { input } = command;
      let output = command.output;
      if (output === undefined) {
        if (input === "-") {
          throw new Failure("usage", "reading standard input, build needs -o");
        }
        const { extname } = await import("node:path");
        output = `${input.slice(0, input.length - extname(input).length)}.xlsx`;
      }
      const source = await readSource(input);
      const inputs = await readInputs(input, source);
      let built;
      try {
        built = await build(source, inputs);
      } catch (error) {
        throw reported(input, error);
      }
      report(input, built.warnings);
      await writeOutput(output, built.xlsx);
      if (output !== "-") {
        process.stderr.write(`yup: wrote ${output}\n`);
      }
      return;
    }
    case "check":
      await showStep(command.input, "check", false);
      return;
    case "show":
      await showStep(command.input, command.step, true);
      return;
  }
}

/**
 * @param {string} input
 * @param {string} step
 * @param {boolean} print
 */
async function showStep(input, step, print) {
  const source = await readSource(input);
  const inputs = await readInputs(input, source);
  let shown;
  try {
    shown = await show(source, step, inputs);
  } catch (error) {
    throw reported(input, error);
  }
  report(input, shown.warnings);
  if (print) {
    process.stdout.write(shown.text);
  }
}

/**
 * Reads the input files the source names, from the folder the source is in.
 *
 * @param {string} input
 * @param {string} source
 */
async function readInputs(input, source) {
  let files;
  try {
    files = await inputFiles(source);
  } catch (error) {
    // A source that does not parse is reported by the steps that follow.
    if (error instanceof YupError) {
      return {};
    }
    throw error;
  }
  const { readFileSync } = await import("node:fs");
  const { dirname, join } = await import("node:path");
  const folder = input === "-" ? "" : dirname(input);
  /** @type {Record<string, string>} */
  const inputs = {};
  /** @type {Diagnostic[]} */
  const problems = [];
  for (const file of files) {
    const path = join(folder, file.name);
    const problem = (/** @type {string} */ message) =>
      problems.push({
        severity: "error",
        file: null,
        line: file.line,
        column: file.column,
        message,
      });
    let bytes;
    try {
      bytes = readFileSync(path);
    } catch (error) {
      problem(`cannot read ${path}: ${describe(error)}`);
      continue;
    }
    const valid = validUpTo(bytes);
    if (valid < bytes.length) {
      problem(`${path} is not valid UTF-8`);
      continue;
    }
    inputs[file.name] = decoder.decode(bytes);
  }
  if (problems.length > 0) {
    report(input, problems);
    throw new Failure("reported");
  }
  return inputs;
}

/** @param {string} input */
async function readSource(input) {
  const { readFileSync } = await import("node:fs");
  let bytes;
  try {
    bytes = readFileSync(input === "-" ? 0 : input);
  } catch (error) {
    throw new Failure("io", `cannot read ${displayName(input)}: ${describe(error)}`);
  }
  const valid = validUpTo(bytes);
  if (valid < bytes.length) {
    const text = decoder.decode(bytes.subarray(0, valid));
    const lines = text.split("\n");
    const last = lines[lines.length - 1];
    report(input, [
      {
        severity: "error",
        file: null,
        line: lines.length,
        column: [...last].length + 1,
        message: "the file is not valid UTF-8 here",
      },
    ]);
    throw new Failure("reported");
  }
  return decoder.decode(bytes);
}

/**
 * How many bytes from the start are valid UTF-8, by the same rules as Rust's: no overlong
 * forms, no surrogates, nothing past U+10FFFF.
 *
 * @param {Uint8Array} bytes
 */
function validUpTo(bytes) {
  let i = 0;
  while (i < bytes.length) {
    const lead = bytes[i];
    if (lead < 0x80) {
      i++;
      continue;
    }
    let follow;
    let low = 0x80;
    let high = 0xbf;
    if (lead >= 0xc2 && lead <= 0xdf) {
      follow = 1;
    } else if (lead === 0xe0) {
      follow = 2;
      low = 0xa0;
    } else if (lead === 0xed) {
      follow = 2;
      high = 0x9f;
    } else if (lead >= 0xe1 && lead <= 0xef) {
      follow = 2;
    } else if (lead === 0xf0) {
      follow = 3;
      low = 0x90;
    } else if (lead >= 0xf1 && lead <= 0xf3) {
      follow = 3;
    } else if (lead === 0xf4) {
      follow = 3;
      high = 0x8f;
    } else {
      return i;
    }
    for (let k = 1; k <= follow; k++) {
      const next = bytes[i + k];
      const [min, max] = k === 1 ? [low, high] : [0x80, 0xbf];
      if (next === undefined || next < min || next > max) {
        return i;
      }
    }
    i += follow + 1;
  }
  return bytes.length;
}

/**
 * Prints each problem after the name of the file it is in.
 *
 * @param {string} input
 * @param {Diagnostic[]} diagnostics
 */
function report(input, diagnostics) {
  for (const diagnostic of diagnostics) {
    process.stderr.write(`${format(diagnostic, displayName(input))}\n`);
  }
}

/**
 * Reports the problems a step stopped at, and gives the failure that says so.
 *
 * @param {string} input
 * @param {unknown} error
 */
function reported(input, error) {
  if (!(error instanceof YupError)) {
    throw error;
  }
  report(input, error.diagnostics);
  return new Failure("reported");
}

/** @param {string} input */
function displayName(input) {
  return input === "-" ? "<stdin>" : input;
}

/**
 * @param {string} output
 * @param {Uint8Array} bytes
 */
async function writeOutput(output, bytes) {
  try {
    if (output === "-") {
      await new Promise((resolve, reject) =>
        process.stdout.write(bytes, (error) => (error ? reject(error) : resolve(undefined))),
      );
    } else {
      const { writeFileSync } = await import("node:fs");
      writeFileSync(output, bytes);
    }
  } catch (error) {
    throw new Failure("io", `cannot write ${output}: ${describe(error)}`);
  }
}

/** @param {any} error */
function describe(error) {
  return error?.message ?? String(error);
}
