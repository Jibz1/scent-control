# Phase 1 test session with the owner

This session checks the desktop app against the real diffusers. It takes about 30 minutes. Note
pass or fail for each step, and anything that surprised you. Claude records the results, with the
date, in `docs/findings.md`.

**Before you start**

- Close the Scentiment app, or turn your phone's Bluetooth off. A diffuser accepts one Bluetooth
  connection at a time.
- Make sure both diffusers are switched on and within a few meters of this Mac.
- Build and open the app from the repo:

```bash
scripts/bundle-macos.sh
```

```bash
open "target/Scent Control.app"
```

Always open the app with `open`, never by running the binary inside it. macOS lets only the app
bundle use Bluetooth.

## 1. The window

1. The title bar blends into the window, and the close, minimise and zoom buttons line up with the
   toolbar's Show Log button and gear.
2. You can drag the window by its top band. Double-clicking the band zooms it.
3. ⌘, opens Settings in its own window. So does the gear.

## 2. Broker

4. In Settings, type the broker password and click Save. Claude never types it for you. Within a
   few seconds the sidebar footer says "Connected to the broker".

## 3. Finding and adding the diffusers

5. Click Scan for Diffusers. macOS asks "Scent Control would like to use Bluetooth": allow it.
   Both diffusers appear under Found Nearby with a signal reading. If the prompt comes back after a
   later rebuild, that's expected for now.
6. Select the HVAC 2 under Found Nearby, click Add Diffuser…, give it a name, and click Add.
   - The steps tick off in order: reading, moving it to your broker, restarting (after a 25 s
     wait), waiting for it to connect, done.
   - Expect it to finish within about 2 minutes. Note how long it took.
   - While it restarts, click Scan for Diffusers once. When it's done, turning it on and off still
     works.
   - While it waits for the diffuser to connect, minimise the window for a moment, then bring it back.
     The add still finishes.
7. Do the same for the Air 2.
8. Within 30 s, both read "Answering" in the detail pane, and the sidebar says what each is doing.

## 4. Control

9. Air 2, with Send Over set to Auto: turn it off and on with the switch, then try Low, Medium and
   High. Check the diffuser does each one. The log (Show Log) says "over Bluetooth".
10. Set Send Over to WiFi and repeat. The log says "over WiFi".
11. Set Send Over back to Auto, then turn the Mac's Bluetooth off in Control Center. Change the
    intensity. Within about 10 s it goes over WiFi, and the log has one line starting "Bluetooth
    couldn't reach the diffuser, so this went over WiFi". Turn Bluetooth back on afterwards.

## 5. A schedule on the Air 2

12. Click Edit Schedule…, then Add a Test Time Starting in 3 Minutes, then Send Schedule. Confirm
    Replace. Leave the diffuser alone.
    - It starts on its own about 3 minutes later and stops when the test time ends.
    - The Today strip and the line under it ("Based on the last schedule sent from Scent Control…") match
      what it does.
    - Note the start and stop times you saw.

## 6. Restart and errors

13. Quit the app (⌘Q) and open it again. Both diffusers are still listed, with their last readings
    and how old they are.
    Open Edit Schedule… on the Air 2: it still shows the schedule you sent in step 12, and the Today
    strip is still there.
14. In Settings, save a wrong broker password. The app says "Broker login refused" and stops
    trying. Then save the right password again, and it connects.
15. Click Show Log and expand a "Pointed … at your broker" line. Its password shows as `***`.

## 7. Passwords on disk

16. Run this. It counts lines in the saved state that mention a password. Expected: `0`.

```bash
grep -ci password ~/Library/Application\ Support/ScentControl/desktop.json
```

## 8. Things to tell Claude

- Any step that failed, or that felt slow or unclear.
- After an add: right-click the log lines around "Credentials saved" and choose Copy, then paste
  them to Claude. Passwords in them are already masked. We still need to learn the exact wording the
  diffuser uses over Bluetooth.
- How the app looks on your screen in light and dark mode.
