# Receiving 978 MHz UAT aircraft

The **ADS-B** workspace receives 1090 MHz Mode S through ez-sdr's local decoder. Its **978 MHz · UAT** option receives live decoded reports from FlightAware **dump978-fa**. The same aircraft table, geographic map, heading-rotated SVG markers, age filters, and trails apply to both sources.

1. Install [dump978-fa](https://github.com/flightaware/dump978) and the SoapySDR driver for your receiver. For an RTL-SDR device, that driver is commonly packaged as `soapysdr-module-rtlsdr`.
2. Select **ADS-B → 978 MHz · UAT**. This stops ez-sdr's current SDR source and audio so the external decoder can open the device.
3. Run this command on the receiver computer:

   ```sh
   dump978-fa --sdr driver=rtlsdr --json-port 127.0.0.1:30979
   ```

4. The panel connects automatically to `127.0.0.1:30979` and retries every two seconds while the decoder starts. The status and valid/invalid/dropped report counts are shown in the sidebar. Use **Disconnect**, edit **Address**, then **Connect** to change the feed. Numeric IPv4 or bracketed IPv6 addresses with a TCP port are supported; hostnames are deliberately not resolved on the UI thread.
5. Selecting **1090 MHz · Mode S** disconnects UAT and restarts the normal source at 1090 MHz / 2.4 MS/s.

For another receiver or a dongle with a specific serial number, use dump978-fa's appropriate `--sdr` settings, such as `--sdr driver=rtlsdr,serial=01234567`. For a remote decoder, configure its JSON listener on an interface reachable from ez-sdr and enter that computer's IP:port. The default loopback command is local only. UAT aircraft broadcasts are primarily available in the United States; a connection with zero reports does not by itself indicate a receiver fault.

The input is dump978-fa's **direct newline JSON stream**, not SkyAware `aircraft.json`, raw UAT frames, or Beast/Mode S data. Reports retain partial updates and only draw map markers after a valid position has arrived. A position expires after the configured age threshold even if nonposition messages continue. Tracks expire after two minutes or the configured age threshold if longer; each receive queue, input line, and track collection has a fixed memory limit. Anonymous and other non-ICAO addresses are tracked separately and are never looked up as ICAO registrations.

Switching to 978 does not install or launch dump978-fa. The external process owns UAT tuning and demodulation; returning to 1090 requires that process to release the same physical dongle first, or use a separate device. Below-sea-level altitudes are currently displayed as zero feet because the common ADS-B table stores unsigned altitude values.

Validation includes recorded-format parser examples, fragmented and oversized stream handling, queue limits, cancellation, report-to-map/table merging, address separation, and position expiry. A loopback TCP lifecycle test exercises actual disconnect/reconnect transport when the environment permits sockets. These tests do not establish reception from physical UAT radio hardware.
