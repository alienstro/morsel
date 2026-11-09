# Release Morsel

Morsel ships as one universal macOS app (Apple Silicon and Intel). A release has three parts:

1. The build: the universal app and the DMG.
2. The Apple signature and the notarization, so that Gatekeeper opens the app on other Macs.
3. The update files, so that each installed Morsel finds and installs the new version.

Without the Apple certificate, the build still works on your own Mac. Other Macs block an app that is not signed and notarized.

## One-time setup

### 1. Apple Developer ID certificate

You need a paid Apple Developer account (99 USD each year).

1. In Xcode, open Settings › Accounts, select your team, and click Manage Certificates.
2. Click "+" and select "Developer ID Application".
3. In Keychain Access, find "Developer ID Application: <your name> (<team id>)".
4. Export the certificate with its private key as a `.p12` file, and give it a strong password.

Write down these values:

| Name | Value |
| --- | --- |
| `APPLE_SIGNING_IDENTITY` | `Developer ID Application: <your name> (<team id>)` |
| `APPLE_TEAM_ID` | The 10-character team ID |
| `APPLE_ID` | The email address of your Apple account |
| `APPLE_PASSWORD` | An app-specific password from account.apple.com › Sign-In and Security |

### 2. The updater key

The app checks each update with a public key. The matching private key signs each update.

CAUTION: Keep the private key and its password safe. If you lose them, the installed apps cannot accept new updates, and each user must download Morsel again.

The public key is in `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`). The private key is in `~/.tauri/morsel-updater.key` on the Mac that made it. Its password is in the login Keychain, service `morsel-updater-key`:

```sh
security find-generic-password -s morsel-updater-key -w
```

To make a new key pair, run `pnpm tauri signer generate -w ~/.tauri/morsel-updater.key`, then put the new public key in `tauri.conf.json`. Apps with the old public key cannot update to a build that the new key signs.

### 3. GitHub secrets

In the repository, open Settings › Secrets and variables › Actions, and add:

| Secret | Value |
| --- | --- |
| `APPLE_CERTIFICATE` | The `.p12` file as base64: `base64 -i cert.p12 \| pbcopy` |
| `APPLE_CERTIFICATE_PASSWORD` | The password of the `.p12` file |
| `APPLE_SIGNING_IDENTITY` | See step 1 |
| `APPLE_ID` | See step 1 |
| `APPLE_PASSWORD` | See step 1 |
| `APPLE_TEAM_ID` | See step 1 |
| `TAURI_SIGNING_PRIVATE_KEY` | The content of `~/.tauri/morsel-updater.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The password from step 2 |

## Publish a release

1. Set the new version in `src-tauri/tauri.conf.json`, `src-tauri/Cargo.toml` and `package.json`.
2. Merge the change to `main`.
3. Push a tag with the same version: `git tag v0.2.0 && git push origin v0.2.0`.
4. The "Release" workflow tests, builds, signs and notarizes the app. Then it makes a draft release with the DMG, the update archive and `latest.json`.
5. Open the draft release on GitHub, write the notes, and click "Publish release".

The installed apps read `https://github.com/alienstro/morsel/releases/latest/download/latest.json`. A draft release is not "latest", so no app updates before you publish.

WARNING: The repository is private, so the app cannot download from its releases. Before the first public release, make the repository public, or put the release files on a public host and change `plugins.updater.endpoints`.

## Build on your own Mac

```sh
pnpm run release
```

The script builds the universal app and the DMG. It signs, notarizes and makes the update files only when the variables of the setup are set in your shell. It ends with the architectures (`x86_64 arm64`) and the signature of the app.
