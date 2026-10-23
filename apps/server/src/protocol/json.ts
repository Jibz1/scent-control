// Reading JSON we didn't write: no exceptions, and integers as serde's `as_i64` reads them.

export type Json = { readonly ok: true; readonly value: unknown } | { readonly ok: false };

export const parseJson = (text: string): Json => {
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch {
    return { ok: false };
  }
};

export const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

export const isInteger = (value: unknown): value is number =>
  typeof value === "number" && Number.isInteger(value);
