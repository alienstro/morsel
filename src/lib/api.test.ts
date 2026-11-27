import { describe, expect, it } from "vitest";
import { errorMessage, needsSettings } from "./api";

// The messages are the real `MorselError` texts from src-tauri/src/types.rs, with their prefixes.
describe("errorMessage", () => {
  it.each([
    ["db", "database error: disk I/O error", "Morsel could not read or save its data. Restart Morsel and try again."],
    ["not_found", "not found: meeting 1", "This note was deleted."],
    ["not_found", "not found: note of meeting 1", "This note was deleted."],
    ["not_found", "not found: template 4", "Morsel could not find this item. It may be deleted."],
    ["invalid", "invalid value: download the transcription model in Settings first", "Download the transcription model in Settings first."],
    ["invalid", "invalid value: A shortcut needs Command, Control or Option.", "A shortcut needs Command, Control or Option."],
    ["invalid_key", "the API key is not valid", "The API key is not valid."],
    ["rate_limit", "the provider rate limit is reached", "The provider rate limit is reached. Wait a minute and try again."],
    ["http", "network or provider error: 502", "Morsel could not reach the provider. Check your internet connection and try again."],
    ["mic_permission_denied", "Morsel has no microphone permission", "Morsel has no microphone permission. Turn it on in System Settings, then try again."],
    ["system_audio_permission_denied", "Morsel has no system audio permission", "Morsel has no system audio permission. Turn it on in System Settings, then try again."],
    ["system_audio_unsupported", "system audio needs macOS 14.2 or later", "System audio needs macOS 14.2 or later."],
    ["audio", "audio error: stream closed", "Morsel could not record the audio. Check your microphone and try again."],
    ["transcribe", "transcription error: bad frame", "Morsel could not transcribe the audio. Try again."],
    ["model_checksum", "the downloaded model is damaged", "The downloaded model is damaged. Download it again in Settings."],
    ["already_recording", "a meeting is already recording", "A meeting is already recording. Stop it first."],
    ["not_recording", "no meeting is recording", "No meeting is recording."],
    ["cancelled", "the request was cancelled", "The request was cancelled."],
    ["keychain", "Keychain error: item locked", "Morsel could not use the Keychain. Unlock your Mac and try again."],
    ["io", "file error: no space left", "Morsel could not read or save a file. Check the free disk space and try again."],
  ])("code %s gives a plain message", (code, message, plain) => {
    expect(errorMessage({ code, message })).toBe(plain);
  });

  it("keeps_the_composio_key_steps", () => {
    const message = "Composio did not accept this key. Morsel needs a project API key that starts with ak_.";
    expect(errorMessage({ code: "invalid_key", message })).toBe(message);
  });

  it("keeps_the_provider_status_and_the_composio_reason", () => {
    expect(errorMessage({ code: "http", message: "network or provider error: HTTP 404: model not found" })).toBe(
      "The provider returned the error HTTP 404. Check the provider settings and try again.",
    );
    expect(errorMessage({ code: "http", message: "network or provider error: Composio sent a sign-in page that is not https." })).toBe(
      "Composio sent a sign-in page that is not https.",
    );
    expect(errorMessage({ code: "http", message: "network or provider error: Composio: no auth config id" })).toBe(
      "Composio: no auth config id.",
    );
  });

  it("keeps_an_error_with_no_code", () => {
    expect(errorMessage(new Error("offline"))).toBe("offline");
    expect(errorMessage("boom")).toBe("boom");
  });
});

describe("needsSettings", () => {
  it("is_true_only_for_a_step_in_settings", () => {
    expect(needsSettings("Download the notes model in Settings first.")).toBe(true);
    expect(needsSettings("Type a question first.")).toBe(false);
  });
});
