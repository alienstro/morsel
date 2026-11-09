import { relaunch } from "@tauri-apps/plugin-process";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { useEffect, useState } from "react";

/** Checks for a new version once, and installs it only when the user clicks. A failed check shows nothing. */
export function UpdateBar() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [state, setState] = useState<"ready" | "installing" | "failed">("ready");

  useEffect(() => {
    let live = true;
    check()
      .then((found) => live && setUpdate(found))
      .catch(() => {});
    return () => {
      live = false;
    };
  }, []);

  if (!update) return null;

  const install = async () => {
    setState("installing");
    try {
      await update.downloadAndInstall();
      await relaunch();
    } catch {
      setState("failed");
    }
  };

  return (
    <div role="status" className="update-bar">
      <span>{state === "failed" ? "The update failed. Try again later." : `Morsel ${update.version} is ready.`}</span>
      {state !== "failed" && (
        <button type="button" className="btn" onClick={install} disabled={state === "installing"}>
          {state === "installing" ? "Updating…" : "Update and restart"}
        </button>
      )}
    </div>
  );
}
