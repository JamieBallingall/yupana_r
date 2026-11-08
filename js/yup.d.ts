// Types for yup.mjs: turns a .yup file and its CSV inputs into an xlsx workbook.
// Licensed under the Apache License, Version 2.0.

/** One problem, with where it was found. */
export interface Diagnostic {
  /** Whether the problem stops the workbook being written. */
  severity: "error" | "warning";
  /** The input file the problem is in, or null for the `.yup` source itself. */
  file: string | null;
  /** The line, counted from 1. */
  line: number;
  /** The column, counted from 1 in characters. */
  column: number;
  /** What the problem is, in a sentence. */
  message: string;
}

/** The input files' contents, by the names the source gives them, such as `history.csv`. */
export type Inputs = Record<string, string> | Map<string, string>;

/** A step from a `.yup` file to a workbook. */
export type Step =
  | "parse"
  | "resolve"
  | "infer"
  | "place"
  | "ground"
  | "check"
  | "lower"
  | "package";

/** The version of the tool. */
export const version: string;

/** The version of the `.yup` language the tool reads. */
export const languageVersion: string;

/** The steps, in order. */
export const steps: readonly Step[];

/** Problems that stopped a step, each with where it is. */
export class YupError extends Error {
  diagnostics: Diagnostic[];
}

/**
 * A diagnostic as the command line prints it, after the name of the file it is in; a
 * diagnostic in the source is given `source`, if it is given.
 */
export function format(diagnostic: Diagnostic, source?: string): string;

/** The input files a source reads, in the order it names them, each with where. */
export function inputFiles(
  source: string,
): Promise<{ name: string; line: number; column: number }[]>;

/** Runs the steps up to Check; throws a YupError for problems. */
export function check(source: string, inputs?: Inputs): Promise<{ warnings: Diagnostic[] }>;

/** Runs the steps up to `step`, and returns what it shows. */
export function show(
  source: string,
  step: Step,
  inputs?: Inputs,
): Promise<{ text: string; warnings: Diagnostic[] }>;

/** Runs every step and returns the workbook's bytes. */
export function build(
  source: string,
  inputs?: Inputs,
): Promise<{ xlsx: Uint8Array; warnings: Diagnostic[] }>;
