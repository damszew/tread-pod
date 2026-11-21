# tread-pod user guide

tread-pod is a small box that sits next to your treadmill. It reads the speed from the treadmill
over Bluetooth and passes it to your watch as a **foot pod**, so your watch records the correct pace
and distance on indoor runs.

You pair your watch with tread-pod **once**. After that it works with any Bluetooth treadmill it
finds, without pairing again.

> This guide covers the current version. Some things (the button, a settings page) are not
> available yet; see [Not available yet](#not-available-yet).

## What you need

- **The tread-pod board** (Seeed Studio XIAO ESP32-S3) **with its antenna attached.** The small
  antenna clips onto the gold connector on the board. Without it, tread-pod barely works.
- **A USB power source:** a phone charger, a laptop, or a USB port on the treadmill.
- **A Bluetooth treadmill.** It must support the standard "FTMS" fitness protocol. Most treadmills
  that work with apps like Zwift or Kinomap do.
- **A watch that supports Bluetooth foot pods.** This guide was tested with a Garmin.

## The light

The yellow light on the board shows the connection to the **treadmill**. Your watch shows its own
connection to tread-pod.

| Light | What it means |
|---|---|
| Off | Your watch isn't using tread-pod right now. This is normal when you're not running. |
| Slow blink (once a second) | Your watch is ready. tread-pod is looking for a treadmill. |
| Fast blink | A treadmill was found, and tread-pod is connecting to it. |
| Steady on | Everything is working. The speed is going to your watch. |

## First-time setup

You don't need to configure anything. tread-pod automatically uses the nearest treadmill.

1. Attach the antenna, then plug tread-pod into USB power close to the treadmill console, ideally
   within a metre or two.
2. Turn the treadmill on so its display is lit.
3. On your watch, add a new sensor. On most Garmins:
   **Settings → Sensors & Accessories → Add New → Foot Pod**, then choose **tread-pod**.
4. Recommended on Garmin, in the foot pod's settings:
   - Set **Speed Mode** (sometimes called **Speed Source**) to **Indoor**, so the watch only uses
     the foot pod on treadmill runs and keeps using GPS outdoors.
   - Set the **Calibration Factor** to **100** and turn automatic calibration off. tread-pod
     already sends the treadmill's exact speed, so it needs no calibration.

## Going for a run

1. Turn the treadmill on.
2. Start a **Treadmill** (or Indoor Run) activity on your watch.
3. Watch the light: it blinks slowly, then quickly, then stays on. That takes a few seconds.
4. Start the belt and run. Pace and distance on the watch follow the treadmill.

When the watch stops using tread-pod, usually shortly after you finish the activity, the light
turns off and tread-pod lets go of the treadmill, so the treadmill's own app can use it again.

## Using a different treadmill

tread-pod picks the treadmill with the strongest signal when your watch starts using it, then
**stays with that treadmill for the whole run**, even if the connection drops for a moment.

- **Moving to another treadmill** (for example, at a gym): finish the activity on your watch, move
  tread-pod next to the other treadmill, and start a new activity.
- **It picked the wrong treadmill:** move tread-pod closer to the treadmill you want, then unplug it
  and plug it back in.

## Sharing tread-pod with another watch

tread-pod works with **one watch at a time**. While someone's watch is using it, no other watch can
connect, so a second runner nearby can't take it over or record the wrong treadmill.

- **If the connection drops mid-run**, tread-pod waits **2 minutes** for that same watch to come
  back. Other watches can't connect during that time.
- **When the activity ends**, tread-pod is free for any watch straight away.
- **If two paired watches are in the same room**, turn the foot pod off on the watch that isn't
  using the treadmill. On Garmin: **Settings → Sensors & Accessories → tread-pod → Status: Off**.
  Otherwise, if both start an activity at the same moment, whichever connects first gets
  tread-pod.

## Troubleshooting

| Problem | Try this |
|---|---|
| Your watch can't find **tread-pod** | Check tread-pod has power. Bring the watch close to it. If the watch is already paired with tread-pod, it won't appear in the list again. If another watch is using it, finish that activity first (see [Sharing tread-pod](#sharing-tread-pod-with-another-watch)). |
| Light keeps **blinking slowly** | tread-pod can't find a treadmill. Make sure the treadmill is on and its display is lit. **Close the treadmill's own app** on your phone: most treadmills accept only one connection. Move tread-pod closer to the treadmill. |
| Light keeps **blinking fast**, or keeps switching between slow and fast | The signal is weak. **Check the antenna is attached** and clicked in. Move tread-pod closer to the treadmill console, and away from metal. |
| Light is **on** but the watch shows no pace | Start the belt: pace only shows while it moves. Check the watch is using the foot pod, not GPS (see Speed Mode in [First-time setup](#first-time-setup)). |
| Pace or distance is a bit off | Set the foot pod's **Calibration Factor** to 100 and turn automatic calibration off. |
| Watch keeps losing the foot pod | The signal is weak: check the antenna and keep tread-pod within a couple of metres of you. |
| Anything else | Unplug tread-pod, wait 5 seconds, and plug it back in. Your watch stays paired. |

## Good to know

- **Unplugging is safe.** You can unplug tread-pod at any time. Your watch stays paired, and the
  pairing survives power cuts.
- **Cadence (steps per minute) and stride length are estimates** worked out from the belt speed,
  using one curve for everyone. Expect them to be a few steps per minute off for very tall or
  short runners. Your watch uses the foot pod's cadence instead of its own, so without an
  estimate it would record 0.
- **A chest strap with running dynamics (such as the HRM 600) doesn't change the cadence.** The
  watch doesn't combine the two: it records the foot pod's estimated cadence even when the strap is
  measuring yours.
- **Firmware version and serial number** are shown on your watch in the foot pod's details
  (on Garmin: select the sensor, then **About**). If they're missing after an update, remove the
  foot pod from the watch and pair it again.
- **The watch works out distance from the speed.** On some treadmills, this can differ a little
  from the treadmill's own display.

## Not available yet

These are planned but don't work in the current version:

- **The button.** Choosing a treadmill with the button, a settings mode and a factory reset are
  still to come.
- **A settings page (Wi-Fi)**, for choosing a fixed treadmill and other options.
- **Sending the treadmill's own distance or incline** to the watch.
