# Calendars: Microsoft 365 and Google

Motiv has no account or server. To read a calendar it signs in to Microsoft or Google in the browser and keeps the sign-in in the computer's keychain (macOS Keychain, Windows Credential Manager). Motiv only reads; nothing is written to anyone's calendar or mailbox.

For that, Microsoft and Google each need to know about the app: an **app registration**. The publisher of Motiv creates **one** of each, puts the IDs in [`frontend/src-tauri/oauth_clients.json`](../frontend/src-tauri/oauth_clients.json), and every copy of Motiv built from the repo uses them. **People using Motiv don't register anything; they click Connect.**

- [For people using Motiv](#for-people-using-motiv)
- [One-time setup for the publisher](#one-time-setup-for-the-publisher)
- [Using your own registration (Advanced)](#using-your-own-registration-advanced)

---

## For people using Motiv

1. **Settings → Accounts → Connect Microsoft 365** (or **Connect Google**).
2. Sign in in the browser and approve "read your calendar".
3. Done. Recordings of calendar meetings are named after the event, and each meeting has a **People** tab with the organizer, invitees and their responses.

What you might see:

| Situation | What happens |
| --- | --- |
| Personal Microsoft account (Outlook.com, Hotmail) | Works directly. |
| Work or school Microsoft account | Works directly if your organisation lets people approve apps themselves. Many require **admin approval** for apps from publishers Microsoft hasn't verified: you'll see "Need admin approval", and your IT admin approves Motiv once for everyone in the organisation. |
| Teams attendance reports (optional checkbox) | Work accounts only, and almost always need admin approval. Turn it on, then **Reconnect**. Reports exist only for Teams meetings you organised, a few minutes after they end (**People → Get attendance**). |
| Google account | Until Google has verified Motiv, Google shows "Google hasn't verified this app": click **Advanced → Go to Motiv**. Limited to 100 users until verified. |

To disconnect: **Settings → Accounts → Disconnect**. To revoke access completely, also remove Motiv at https://myaccount.microsoft.com (Apps & services) or https://myaccount.google.com/permissions.

---

## One-time setup for the publisher

Uses your **personal** Microsoft and Google accounts; no company is involved. About 10 minutes each.

### Microsoft (Entra ID)

1. Sign in to **https://portal.azure.com** with your personal Microsoft account. If you don't have an Azure account yet, create the free one; it gives you your own directory ("Default Directory").
2. Search for **App registrations** → **New registration**:
   - **Name:** `Motiv`
   - **Supported account types:** **Accounts in any organizational directory and personal Microsoft accounts**
   - **Redirect URI:** platform **Public client/native (mobile & desktop)**, URI `http://localhost`
     (Motiv listens on a random local port; Microsoft accepts any port for `localhost`.)
   - **Register**.
3. **API permissions** → **Add a permission** → **Microsoft Graph** → **Delegated permissions**:
   `User.Read`, `Calendars.Read`, `offline_access`, `openid`, `profile`, `email`,
   and, for the optional attendance reports, `OnlineMeetings.Read` and `OnlineMeetingArtifact.Read.All`.
   You don't grant admin consent here; each organisation consents for itself.
4. **Branding & properties:** add a name, and optionally a logo, a home page (your GitHub repo) and a privacy statement URL. These appear on the consent screen.
5. Copy the **Application (client) ID** from **Overview**.

### Google (Google Cloud)

1. Go to **https://console.cloud.google.com** with your personal Google account and create a project named `Motiv`.
2. **APIs & Services → Library** → **Google Calendar API** → **Enable**.
3. **Google Auth Platform**:
   - **Branding:** app name `Motiv`, your email, optional logo.
   - **Audience:** **External**, then **Publish app**. (In *Testing* mode, sign-ins expire after 7 days.)
   - **Data access:** add `openid`, `.../auth/userinfo.email`, `.../auth/calendar.readonly`.
4. **Clients** → **Create client** → **Desktop app** → `Motiv` → **Create**. Copy the **Client ID** and **Client secret**.
5. Optional, to remove the "unverified app" warning and the 100-user cap: submit the app for **verification**. `calendar.readonly` is a *sensitive* scope; Google asks for a home page, a privacy policy, and a short video showing the sign-in and how the data is used.

### Build the IDs into Motiv

Edit `frontend/src-tauri/oauth_clients.json`:

```json
{
  "microsoft": { "client_id": "<Application (client) ID>", "tenant": "common" },
  "google": { "client_id": "<Client ID>", "client_secret": "<Client secret>" }
}
```

Commit it and rebuild. These values identify the app and are not secrets: a desktop app's client ID ships inside the app anyway, and Google treats a Desktop client's "secret" as non-confidential. The sign-in itself is protected by PKCE.

Microsoft's **publisher verification** (the blue "verified" badge that lets many organisations approve apps without an admin) requires a Microsoft business partner ID, so it isn't available to a personal publisher. That's why some work accounts will need their admin to approve Motiv.

---

## Using your own registration (Advanced)

Organisations that don't allow third-party apps can register Motiv in their own directory and enter it in **Settings → Accounts → Advanced**:

- **Microsoft:** follow the publisher steps above, but choose **Accounts in this organizational directory only**, then click **Grant admin consent**. In Motiv, enter the Application (client) ID and the tenant (directory ID or domain).
- **Google:** follow the publisher steps with **Audience: Internal** (Google Workspace). Enter the Client ID and Client secret.

Leaving the Advanced fields empty uses the built-in registration.

---

Zoom participant lists (Zoom's own API) are not connected yet. Zoom meetings are matched through calendar events that contain a Zoom link.
