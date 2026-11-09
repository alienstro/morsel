type Internals = { __TAURI_INTERNALS__?: { metadata?: { currentWindow?: { label?: string } } } };

/** The label of this Tauri window. Tests and plain browsers get "main". */
export function windowLabel(): string {
  return (window as unknown as Internals).__TAURI_INTERNALS__?.metadata?.currentWindow?.label ?? "main";
}
