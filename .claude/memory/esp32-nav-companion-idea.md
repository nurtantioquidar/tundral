---
name: esp32-nav-companion-idea
description: 2026-09-21 the user flagged a viral Indonesian DIY build — ESP32 + 2.4" TFT BLE companion display that keeps an ojol rider's AMOLED phone off during navigation
metadata:
  type: project
---

On 2026-09-21 the user shared (as a screenshot, "check this, learn that") a Facebook post by
Tedy Ramdani in the group "Keluh Kesah Kehidupan Berteknologi". An unemployed rider turned to
ojol, found that Maps navigation forces the phone to stay awake, and refused to burn an AMOLED
panel for small earnings. He built a ~Rp100k navigator: ESP32 + 2.4" TFT in a 3D-printed case,
firmware labelled `ESP32-NAV-FD8C v1.0.0`, UI of four buttons — Navigate / Notify / Settings /
Info — over a status line reading "Connected and synced, Samsung SM-G?98G".

Claimed: works like Google Maps on a smartwatch, phone can sleep while GPS/map/compass keep
running, offline map and offline "GPS", best route by distance (OSM, so no traffic-aware
choice), accepts a Maps app handoff or a coordinate link, mirrors incoming notifications with
their app icon, phone stays cool in a pocket. Only stated flaw: dim screen, still glanceable.
Framed against the alternative of a Rp500k used phone.

What it actually is, read against the price and the screen: a **BLE companion display**, not a
standalone navigator. Rp100k does not cover a GNSS module on top of an ESP32 and a TFT, and
"Connected and synced <phone model>" means the phone supplies the fix, the route, and the
notifications while the ESP32 only renders. "GPS offline" means no data connection needed
(phone GNSS + offline OSM tiles), not an onboard receiver. Be precise about that if this idea
is ever picked up — the tether is the whole architecture, and the compass is probably GNSS
course, not a magnetometer.

The transferable lesson, and why it fits the Monet thesis of small single-purpose tools
(see [[monet-product-line-structure]]): a cheap dedicated surface that offloads one always-on
job from an expensive general device beats replacing the expensive device. Same shape as
[[cicada-product]] — don't open a browser console to read a balance.

Status: an idea the user flagged, nothing more. No workspace, no decision, and per the
monorepo guide it would only become a product with its own audience and brand.
