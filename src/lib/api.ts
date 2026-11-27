import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Speaker = "you" | "others";
export type MeetingStatus =
  | "recording" | "paused" | "stopped" | "transcribing" | "writing_notes" | "done"
  | "notes_cancelled" | "notes_failed" | "interrupted";

export type TranscriptLine = { meetingId: number; speaker: Speaker; startMs: number; endMs: number; text: string };
export type MeetingSummary = {
  id: number; title: string; templateId: number; templateName: string; status: MeetingStatus; startedAt: number;
  endedAt: number | null; people: string[];
};
export type MeetingDetail = { summary: MeetingSummary; lines: TranscriptLine[]; userNotes: string; note: string | null };
export type Section = { name: string; hint: string };
export type TemplateInput = { name: string; description: string; sections: Section[]; instructions: string };
export type Template = TemplateInput & { id: number; builtIn: boolean; uses: number };
export type NotesFilter = {
  query?: string | null; templateId?: number | null; person?: string | null; from?: number | null; to?: number | null;
  limit: number; offset: number;
};
export type NotesPage = { items: MeetingSummary[]; total: number };
export type AskRange = "all" | "week" | "month";
export type AskSource = { meetingId: number; title: string; startedAt: number };
export type AskAnswer = { id: number | null; question: string; answer: string; sources: AskSource[] };
export type UsageTotal = { promptTokens: number; completionTokens: number; cost: number | null };
export type DataInfo = { path: string; notes: number; bytes: number };
export type Settings = {
  notesMode: "local" | "cloud" | null; notesBaseUrl: string; notesModel: string; quickModel: string; transcribeMode: "local" | "cloud"; transcribeTiming: "live" | "after";
  transcribeBaseUrl: string; transcribeModel: string; language: string;
  onboarded: boolean; remindBeforeMeeting: boolean; globalShortcut: string;
};
export type MicAccess = "allowed" | "denied" | "unknown";
export type PrivacyPane = "microphone" | "audio_capture";
export type KeyAccount = "notes_api_key" | "transcribe_api_key" | "composio_api_key";
export type Provider = "google" | "outlook";
export type LinkState = "none" | "pending" | "connected";
export type ProviderStatus = { state: LinkState; today: number };
export type CalendarStatus = { hasKey: boolean; google: ProviderStatus; outlook: ProviderStatus };
export type CalendarEvent = {
  id: string; provider: Provider; title: string; start: number; end: number; link: string | null; people: string[];
};
export type MorselError = { code: string; message: string };

export type StatusEvent = { meetingId: number; status: MeetingStatus; warning: string | null };
export type LevelsEvent = { you: number; others: number };

/** One function for each backend command. Arguments are camelCase; Tauri maps them to snake_case. */
export const api = {
  startMeeting: (title: string | null, templateId: number, people?: string[]) =>
    invoke<number>("start_meeting", { title, templateId, people }),
  pauseMeeting: () => invoke<void>("pause_meeting"),
  resumeMeeting: () => invoke<void>("resume_meeting"),
  /** Returns the timing of the meeting. In "after" mode, Morsel writes the notes by itself. */
  stopMeeting: () => invoke<Settings["transcribeTiming"]>("stop_meeting"),
  saveUserNotes: (meetingId: number, body: string) => invoke<void>("save_user_notes", { meetingId, body }),
  writeNotes: (meetingId: number, templateId: number) => invoke<void>("write_notes", { meetingId, templateId }),
  cancelNotes: (meetingId: number) => invoke<boolean>("cancel_notes", { meetingId }),
  getMeeting: (meetingId: number) => invoke<MeetingDetail>("get_meeting", { meetingId }),
  listMeetings: (limit: number) => invoke<MeetingSummary[]>("list_meetings", { limit }),
  listTemplates: () => invoke<Template[]>("list_templates"),
  getSettings: () => invoke<Settings>("get_settings"),
  saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
  setApiKey: (account: KeyAccount, key: string) => invoke<void>("set_api_key", { account, key }),
  hasApiKey: (account: KeyAccount) => invoke<boolean>("has_api_key", { account }),
  testNotesKey: () => invoke<void>("test_notes_key"),
  modelStatus: () => invoke<boolean>("model_status"),
  downloadModel: () => invoke<void>("download_model"),
  notesModelStatus: () => invoke<boolean>("notes_model_status"),
  downloadNotesModel: () => invoke<void>("download_notes_model"),
  searchModelStatus: () => invoke<boolean>("search_model_status"),
  downloadSearchModel: () => invoke<void>("download_search_model"),
  /** The size in bytes of the saved audio of a meeting, or null when it has none. */
  audioSize: (meetingId: number) => invoke<number | null>("audio_size", { meetingId }),
  deleteAudio: (meetingId: number) => invoke<void>("delete_audio", { meetingId }),
  exportNote: (meetingId: number, path: string) => invoke<void>("export_note", { meetingId, path }),
  searchNotes: (filter: NotesFilter) => invoke<NotesPage>("search_notes", { filter }),
  listPeople: () => invoke<string[]>("list_people"),
  deleteMeeting: (meetingId: number) => invoke<void>("delete_meeting", { meetingId }),
  saveTemplate: (id: number | null, input: TemplateInput) => invoke<number>("save_template", { id, input }),
  duplicateTemplate: (id: number) => invoke<number>("duplicate_template", { id }),
  deleteTemplate: (id: number) => invoke<void>("delete_template", { id }),
  resetTemplate: (id: number) => invoke<void>("reset_template", { id }),
  ask: (question: string, range: AskRange, meetingId: number | null) => invoke<AskAnswer>("ask", { question, range, meetingId }),
  listAsks: (limit: number) => invoke<AskAnswer[]>("list_asks", { limit }),
  getAsk: (id: number) => invoke<AskAnswer>("get_ask", { id }),
  usageSummary: () => invoke<UsageTotal>("usage_summary"),
  dataInfo: () => invoke<DataInfo>("data_info"),
  exportAll: (dir: string) => invoke<number>("export_all", { dir }),
  deleteAllData: () => invoke<void>("delete_all_data"),
  microphoneAccess: () => invoke<MicAccess>("microphone_access"),
  requestMicrophone: () => invoke<MicAccess>("request_microphone"),
  /** Records system audio for a moment, so that macOS asks the user and lists Morsel in its settings. */
  requestSystemAudio: () => invoke<void>("request_system_audio"),
  openPrivacySettings: (pane: PrivacyPane) => invoke<void>("open_privacy_settings", { pane }),
  calendarStatus: () => invoke<CalendarStatus>("calendar_status"),
  connectCalendar: (provider: Provider) => invoke<void>("connect_calendar", { provider }),
  disconnectCalendar: (provider: Provider) => invoke<void>("disconnect_calendar", { provider }),
  refreshCalendar: () => invoke<void>("refresh_calendar"),
  upcomingEvents: () => invoke<CalendarEvent[]>("upcoming_events"),
  reminderEvent: () => invoke<CalendarEvent | null>("reminder_event"),
  startFromReminder: () => invoke<number>("start_from_reminder"),
  dismissReminder: () => invoke<void>("dismiss_reminder"),
  setGlobalShortcut: (shortcut: string) => invoke<void>("set_global_shortcut", { shortcut }),
};

function on<T>(name: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(name, (event) => handler(event.payload));
}

export const onTranscript = (h: (line: TranscriptLine) => void) => on("meeting://transcript", h);
export const onLevels = (h: (levels: LevelsEvent) => void) => on("meeting://levels", h);
export const onStatus = (h: (status: StatusEvent) => void) => on("meeting://status", h);
export const onNotesDone = (h: (e: { meetingId: number }) => void) => on("notes://done", h);
export const onNotesFailed = (h: (e: { meetingId: number; message: string }) => void) => on("notes://failed", h);
export const onOpenMeeting = (h: (e: { id: number }) => void) => on("app://open-meeting", h);
export const onCalendarChanged = (h: () => void) => on("calendar://changed", h);
export const onTranscribeProgress = (h: (e: { meetingId: number; doneMs: number; totalMs: number }) => void) =>
  on("transcribe://progress", h);
export const onModelDownload = (h: (e: { received: number; total: number }) => void) => on("model://download", h);
export const onNotesModelDownload = (h: (e: { received: number; total: number }) => void) => on("notes-model://download", h);
export const onSearchModelDownload = (h: (e: { received: number; total: number }) => void) => on("search-model://download", h);
/** The search index: the meetings or parts done so far, and the total. */
export const onSearchProgress = (h: (e: { done: number; total: number }) => void) => on("search://progress", h);

/** The plain text for each `MorselError` code whose backend text is internal (issue #24). */
const PLAIN_TEXT: Record<string, string> = {
  db: "Morsel could not read or save its data. Restart Morsel and try again.",
  not_found: "Morsel could not find this item. It may be deleted.",
  rate_limit: "The provider rate limit is reached. Wait a minute and try again.",
  http: "Morsel could not reach the provider. Check your internet connection and try again.",
  mic_permission_denied: "Morsel has no microphone permission. Turn it on in System Settings, then try again.",
  system_audio_permission_denied: "Morsel has no system audio permission. Turn it on in System Settings, then try again.",
  audio: "Morsel could not record the audio. Check your microphone and try again.",
  transcribe: "Morsel could not transcribe the audio. Try again.",
  model_checksum: "The downloaded model is damaged. Download it again in Settings.",
  already_recording: "A meeting is already recording. Stop it first.",
  keychain: "Morsel could not use the Keychain. Unlock your Mac and try again.",
  io: "Morsel could not read or save a file. Check the free disk space and try again.",
};

/** The text to show for any error from a command. */
export function errorMessage(error: unknown): string {
  if (!error || typeof error !== "object" || !("message" in error)) return String(error);
  const { code, message } = error as MorselError;
  if (code === "not_found" && /meeting \d+$/.test(message)) return "This note was deleted.";
  if (code === "http") {
    // Issue #24: a provider status or a Composio reason is not a network fault, so keep it.
    const detail = String(message).replace(/^network or provider error: /, "");
    const status = /^HTTP (\d+)/.exec(detail)?.[1];
    if (status) return `The provider returned the error HTTP ${status}. Check the provider settings and try again.`;
    if (detail.startsWith("Composio")) return detail.charAt(detail.length - 1) === "." ? detail : `${detail}.`;
  }
  if (code && PLAIN_TEXT[code]) return PLAIN_TEXT[code];
  if (!code) return String(message);
  // Issue #24: the other codes carry a sentence for the user, so drop the `invalid value: ` prefix.
  const text = String(message).replace(/^invalid value: /, "");
  return text.charAt(0).toUpperCase() + text.slice(1) + (/[.!?]$/.test(text) ? "" : ".");
}

// shortcut: matches the backend text, as no error code marks a missing setup; add a code if more screens need it.
/** True when the error text tells the user to change something in Settings → Models. */
export const needsSettings = (text: string) => text.includes(" in Settings");

/** `m:ss`, or `h:mm:ss` from one hour, from milliseconds. */
export function clock(ms: number): string {
  const total = Math.floor(ms / 1000);
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor(total / 60) % 60;
  const seconds = String(total % 60).padStart(2, "0");
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, "0")}:${seconds}` : `${minutes}:${seconds}`;
}
