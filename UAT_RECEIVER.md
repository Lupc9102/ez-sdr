# Receiving 978 MHz UAT aircraft

The **ADS-B** workspace decodes 1090 MHz Mode S and 978 MHz UAT locally from the shared IQ capture. The **Native IQ** source is selected by default. It tunes to **978 MHz** and requires a capture clock of exactly **2,083,333 complex samples/s**. The same aircraft table, geographic map, heading-rotated SVG markers, age filters, and trails apply to both aircraft sources.

## Native IQ (default)

Select **ADS-B → 978 MHz · UAT → Native IQ**, then choose **Start native UAT**. The local decoder consumes EZ-SDR's capture stream; no separate decoder or JSON feed is needed. A missing/stopped capture source or an incompatible center frequency/sample rate is reported in the receiver status. Start the configured receiver from the Radio controls, or select an external feed below if another process owns the SDR.

## Optional dump978-fa JSON compatibility

To use a separately running [dump978-fa](https://github.com/flightaware/dump978), select **dump978-fa JSON** in the UAT source controls. Install dump978-fa and the SoapySDR driver for your receiver. For an RTL-SDR device, that driver is commonly packaged as `soapysdr-module-rtlsdr`. Run this on the receiver computer:

```sh
dump978-fa --sdr driver=rtlsdr --json-port 127.0.0.1:30979
```

Enter `127.0.0.1:30979` in **Address** and select **Connect**. The status and valid/invalid/dropped report counts are shown in the sidebar. Use **Disconnect**, edit **Address**, then **Connect** to change the feed. Numeric IPv4 or bracketed IPv6 addresses with a TCP port are supported; hostnames are deliberately not resolved on the UI thread. The default loopback address works only when dump978-fa runs on the same computer.

For another receiver or a dongle with a specific serial number, use dump978-fa's appropriate `--sdr` settings, such as `--sdr driver=rtlsdr,serial=01234567`. For a remote decoder, configure its JSON listener on an interface reachable from EZ-SDR and enter that computer's numeric IP:port. External JSON mode releases EZ-SDR's local SDR source so dump978-fa can own the device; stop dump978-fa before returning that same dongle to local capture. UAT aircraft broadcasts are primarily available in the United States; a connection with zero reports does not by itself indicate a receiver fault.

The input is dump978-fa's **direct newline JSON stream**, not SkyAware `aircraft.json`, raw UAT frames, or Beast/Mode S data. Reports retain partial updates and only draw map markers after a valid position has arrived. A position expires after the configured age threshold even if nonposition messages continue. Tracks expire after two minutes or the configured age threshold if longer; each receive queue, input line, and track collection has a fixed memory limit. Anonymous and other non-ICAO addresses are tracked separately and are never looked up as ICAO registrations.

Selecting **1090 MHz · Mode S** returns to local 1090 MHz / 2.4 MS/s decoding. Below-sea-level altitudes are currently displayed as zero feet because the common ADS-B table stores unsigned altitude values.

Validation includes recorded-format parser examples, fragmented and oversized stream handling, queue limits, cancellation, report-to-map/table merging, address separation, and position expiry. A loopback TCP lifecycle test exercises actual disconnect/reconnect transport when the environment permits sockets. These tests do not establish reception from physical UAT radio hardware.
