# Connecting calendars (Microsoft 365 and Google)

Motiv has no account or server. To read your calendar it signs in to Microsoft or Google in your browser, using an **app registration that you own**, and keeps the tokens in your computer's keychain (macOS Keychain, Windows Credential Manager). Each registration takes about 10 minutes and is done once.

What Motiv reads, and only from your computer:

| | Microsoft 365 | Google |
| --- | --- | --- |
| Calendar events, organizer, attendees | `Calendars.Read` | `calendar.readonly` |
| Your name and email (shown in Settings) | `User.Read`, `openid`, `profile`, `email` | `openid`, `email` |
| Teams meeting lookup | `OnlineMeetings.Read` | |
| Teams attendance reports (optional) | `OnlineMeetingArtifact.Read.All` | |
| Stay signed in | `offline_access` | offline access |

Nothing is written to your calendar or mailbox.

---

## Microsoft 365 (Entra ID)

1. Go to **https://entra.microsoft.com** → **Applications** → **App registrations** → **New registration**.
   - **Name:** `Motiv`
   - **Supported account types:** *Accounts in this organizational directory only* (recommended for your own tenant).
   - **Redirect URI:** platform **Public client/native (mobile & desktop)**, URI `http://localhost`
     (Motiv listens on a random local port; Microsoft accepts any port for `localhost`.)
   - Click **Register**.
2. On the app's **Overview**, copy the **Application (client) ID** and the **Directory (tenant) ID**.
3. **API permissions** → **Add a permission** → **Microsoft Graph** → **Delegated permissions**, add:
   `User.Read`, `Calendars.Read`, `OnlineMeetings.Read`, `offline_access`, `openid`, `profile`, `email`,
   and, for attendance reports, `OnlineMeetingArtifact.Read.All`.
4. Click **Grant admin consent for _your organisation_** (needs an admin; `OnlineMeetingArtifact.Read.All` always does).
5. In Motiv: **Settings → Accounts → Microsoft 365**
   - **Application (client) ID:** from step 2
   - **Tenant:** your Directory (tenant) ID or your domain (e.g. `whitedog.com`). Use `organizations` only if the app is multi-tenant.
   - Leave **Teams attendance reports** on if you granted that permission.
   - Click **Connect** and finish signing in in the browser.

Notes
- Attendance reports exist only for Teams meetings **you organised**, and Teams creates them a few minutes after the meeting ends. Use **People → Get attendance** in the meeting.
- If sign-in says the app needs admin approval, step 4 hasn't been done, or turn off **Teams attendance reports** and connect again.

---

## Google Calendar (Google Cloud)

1. Go to **https://console.cloud.google.com** and create a project named `Motiv` (or pick one).
2. **APIs & Services → Library** → search **Google Calendar API** → **Enable**.
3. **Google Auth Platform** (OAuth consent screen):
   - **Audience:** *Internal* if you use Google Workspace (only your organisation can sign in); otherwise *External*.
   - **Data access:** add the scopes `openid`, `.../auth/userinfo.email` and `.../auth/calendar.readonly`.
   - If you chose *External*, either add yourself under **Test users** or **Publish** the app. In *Testing* mode Google expires the sign-in after 7 days, and you'd have to reconnect weekly.
4. **Clients** (Credentials) → **Create client** → **Application type: Desktop app** → name `Motiv` → **Create**. Copy the **Client ID** and **Client secret**.
   (Desktop clients allow any local `http://localhost:<port>` redirect; nothing else to configure.)
5. In Motiv: **Settings → Accounts → Google**, paste the Client ID and Client secret, click **Connect**, and finish in the browser.

The "client secret" of a Desktop app isn't confidential in Google's model; it identifies the app, and the sign-in is protected by PKCE.

---

## What you get

- **Meeting names:** when Motiv detects a Zoom, Teams or Meet call during a calendar event, the recording is named after the event.
- **People tab** in each meeting: the matching calendar event (picked automatically by time; you can choose another), organizer, invitees and their responses, and Teams attendance.
- **Better notes:** participant names are given to summaries, chat and action items so names and owners are spelled correctly.

## Disconnecting

**Settings → Accounts → Disconnect** removes the tokens from the keychain. To revoke access entirely, also remove Motiv from https://myaccount.microsoft.com (Apps & services) or https://myaccount.google.com/permissions, or delete the app registration.

Zoom participant lists (Zoom's own API) are not connected yet; Zoom meetings are matched through calendar events that contain a Zoom link.
