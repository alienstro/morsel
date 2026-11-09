import { vi } from "vitest";

type Handler = (event: { payload: unknown }) => void;

/** Fake Tauri IPC for tests. `invoke` answers from `responses`; `emit` calls the listeners. */
export const handlers: Record<string, Handler[]> = {};
export const responses: Record<string, unknown> = {};

export const invoke = vi.fn(async (cmd: string, _args?: Record<string, unknown>) => {
  const value = responses[cmd];
  return typeof value === "function" ? (value as (a?: unknown) => unknown)(_args) : value;
});

export const listen = vi.fn(async (name: string, handler: Handler) => {
  (handlers[name] ??= []).push(handler);
  return () => {
    handlers[name] = (handlers[name] ?? []).filter((h) => h !== handler);
  };
});

export function emit(name: string, payload: unknown) {
  for (const handler of handlers[name] ?? []) handler({ payload });
}

export function resetTauri() {
  invoke.mockClear();
  listen.mockClear();
  for (const key of Object.keys(handlers)) delete handlers[key];
  for (const key of Object.keys(responses)) delete responses[key];
}
