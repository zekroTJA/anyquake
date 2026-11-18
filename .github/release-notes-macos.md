## Installing on macOS

AnyQuake is not notarized by Apple, so macOS blocks it the first time you start it. It also needs the Accessibility permission to register the global hotkey and manage windows.

The quickest way to install or update on macOS (and Linux) is the install script. It skips the Gatekeeper prompt and resets the outdated Accessibility permission on updates, so you only have to add AnyQuake under Accessibility afterwards:

```sh
curl -fsSL https://raw.githubusercontent.com/zekroTJA/anyquake/main/install.sh | bash
```

To install from the `.dmg` by hand, follow these steps.

### First installation

1. Download `AnyQuake-<version>.dmg`, open it and drag **AnyQuake** into **Applications**.
2. Start **AnyQuake** from Applications. macOS shows a warning that it can't verify the app. Click **Done** (or **OK**).
3. Open **System Settings → Privacy & Security** and scroll down to **Security**. Next to the message about AnyQuake being blocked, click **Open Anyway**, then confirm with your password or Touch ID.
4. Start **AnyQuake** again and click **Open Anyway** in the dialog that appears.
5. Open **System Settings → Privacy & Security → Accessibility**.
6. Click **+**, select **AnyQuake** in Applications and click **Open**. Make sure the switch next to AnyQuake is on.
7. Quit AnyQuake from its menu bar icon and start it again.

### Updating

macOS ties the Accessibility permission to the exact app build, so the old entry stops working after an update even though it still looks enabled. Remove it and add it again:

1. Quit AnyQuake from its menu bar icon.
2. Open the new `AnyQuake-<version>.dmg` and drag **AnyQuake** into **Applications**, replacing the old version.
3. If macOS blocks the new version on first start, allow it again as in steps 2–4 above (**Privacy & Security → Open Anyway**).
4. Open **System Settings → Privacy & Security → Accessibility**.
5. Select the existing **AnyQuake** entry and click **−** to remove it.
6. Click **+**, select **AnyQuake** in Applications and click **Open**. Make sure the switch next to AnyQuake is on.
7. Start AnyQuake again.

---

