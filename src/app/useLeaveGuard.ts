import { useEffect, useState } from "react";
import { useLocation, useNavigate } from "react-router";

/** Stops a link click that leaves the page while `dirty`, and keeps the target until the user decides. */
export function useLeaveGuard(dirty: boolean) {
  const [leaveTo, setLeaveTo] = useState<string | null>(null);
  const navigate = useNavigate();
  const { pathname } = useLocation();
  // BrowserRouter has no useBlocker, so catch the link click before react-router does (#22).
  useEffect(() => {
    if (!dirty) return;
    const onClick = (e: MouseEvent) => {
      const href = (e.target as Element).closest?.("a[href]")?.getAttribute("href");
      if (!href || href.split(/[?#]/)[0] === pathname) return;
      e.preventDefault();
      setLeaveTo(href);
    };
    document.addEventListener("click", onClick, true);
    return () => document.removeEventListener("click", onClick, true);
  }, [dirty, pathname]);
  return { leaveTo, leave: () => leaveTo && navigate(leaveTo), stay: () => setLeaveTo(null) };
}
