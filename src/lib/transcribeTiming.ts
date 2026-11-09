import type { Settings } from "./api";

/** The two choices of "When to transcribe", in the order that the screens show them. */
export const TIMING_CHOICES: { value: Settings["transcribeTiming"]; title: string; about: string }[] = [
  { value: "live", title: "While the meeting runs", about: "The transcript grows during the call." },
  {
    value: "after",
    title: "After the meeting",
    about: "The Mac stays fast during the call. A 1-hour meeting takes about 20 minutes after you stop.",
  },
];
