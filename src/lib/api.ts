import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type Speaker = "you" | "others";
export type MeetingStatus =
  | "recording" | "paused" | "stopped" | "writing_notes" | "done"
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
  notesBaseUrl: string; notesModel: string; quickModel: string; transcribeMode: "local" | "cloud";
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
  stopMeeting: () => invoke<void>("stop_meeting"),
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
export const onModelDownload = (h: (e: { received: number; total: number }) => void) => on("model://download", h);

/** The text to show for any error from a command. */
export function errorMessage(error: unknown): string {
  if (error && typeof error === "object" && "message" in error) return String((error as MorselError).message);
  return String(error);
}

/** `m:ss` from milliseconds. */
export function clock(ms: number): string {
  const total = Math.floor(ms / 1000);
  return `${Math.floor(total / 60)}:${String(total % 60).padStart(2, "0")}`;
}
