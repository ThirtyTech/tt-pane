---
title: Resize & refine
description: Learn anchored growth, width adjustments, gap filling, and paired vertical resizing.
---

## Repeat to grow

Press the same positioning shortcut on the same window to cycle through:

**Place → 115% → 132.25% → 152.0875% → original placement.**

These percentages describe each dimension relative to the initial placement, not area. Each repeat multiplies width and height by 1.15, rounded to pixels and capped at the monitor work area. A full dimension stays full.

Corners stay anchored. Bottom left grows upward and right; left and right halves grow inward; center grows around its midpoint. Fill work area stays full throughout the cycle.

In **Settings → Advanced → Repeat shortcut to grow**, adjust **Growth per press** (1–100%) and **Growth steps before reset** (0–20). Zero steps disables growth. Changing action, window, monitor work area, or growth settings starts a new cycle. External moves or resizes also reset it, allowing two pixels for platform rounding. There is no timeout.

## Adjust only the width

**Ctrl + Alt + -** shrinks the current width by 25%; **Ctrl + Alt + =** expands it by 25%. Height and vertical position stay fixed.

The last matching placement supplies the anchor: left positions keep the left edge, right positions keep the right edge, and centered positions keep the midpoint. A manually placed window’s horizontal position determines its anchor.

Width adjustments do not cycle or reset. Expansion stops at the monitor boundary; application minimum widths can stop shrinking. Each change uses the current width, so shrinking and expanding are not inverse operations: **1000 → 750 → 938 pixels**.

Change the shared percentage in **Advanced → Width adjustment → Width change per press** (1–99%). It is separate from repeated-placement growth.

## Fill the gap

1. Position a window, for example in the bottom-left quarter.
2. Shrink its width with **Ctrl + Alt + -**.
3. Select a **different window on the same monitor**.
4. Press **Ctrl + Alt + Shift + 8** to fill the freed space.

A 1000-pixel-wide slot reduced to 750 pixels leaves a 250-pixel-wide gap to its right. Repeated shrinking accumulates space in the original slot. One gap is remembered per monitor; successfully filling it consumes it. For a centered window, the larger gap wins, with the right side winning ties.

Gaps come from tt-pane’s own resizing, not a scan of empty desktop space. Manual dragging, resizing, or closing outside tt-pane is not monitored: make a fresh width adjustment to update the gap. Selecting the source window or filling without a saved gap reports an error without repositioning it.

## Move a shared boundary — Windows

**Ctrl + Alt + ↑ / ↓** moves a shared horizontal boundary by 25% of the selected window’s current height.

With the bottom window selected, Up grows it upward and shrinks its neighbor. A 50/50 split becomes 62.5/37.5. Down reverses the direction. With the top window selected, Up shrinks it and grows the bottom window.

There is a **50/50 stop** when approaching or crossing equal heights; another press continues past it. Widths, outside edges, and existing gaps stay fixed. The step is independent of the width and growth settings.

The neighbor must be visible, restored, on the same monitor, and have matching horizontal edges within two pixels. Up to 16 pixels of vertical spacing and two pixels of border-rounding overlap are accepted. With neighbors on both sides, the arrow chooses the side; with one neighbor, both arrows adjust that pair.

Missing or ambiguous neighbors produce an error. If an application refuses the size, tt-pane attempts to restore both original bounds and reports the error. Successful adjustments clear saved growth and gap state.
