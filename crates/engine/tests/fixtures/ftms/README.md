# FTMS captures

Raw traffic from real treadmills, recorded with `sniff` (`tools/ftms-sniff`), used by `engine`'s
parser tests. One capture per file:

```text
# metadata: device, RSSI, unix time, note, which characteristics exist
<ms since subscribe> <16-bit characteristic UUID, hex> <payload hex>
```

Reads of static characteristics (feature `2acc`, supported speed range `2ad4`, …) are logged at
`0`; notifications (treadmill data `2acd`, status `2ada`, training status `2ad3`) follow in order.

When recording, cover at least: standing still, a few speed changes, incline changes if the
treadmill supports them, and stopping. Note what you did with `--note`.
