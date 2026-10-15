# Meteor synthetic transport fixture

`meteor-gray-segment.rs255.bin` is a 1020-byte, depth-four interleaved
RS(255,223) codeword generated for the Meteor offline decoder integration test.
It is a synthetic fixture, not a satellite recording or proof of RF performance.

The 892-byte transport payload contains a six-byte AOS VCDU header (spacecraft
1, virtual channel 5, sequence 0), a two-byte insert zone, a zero M-PDU first
header pointer, and an unsegmented APID 65 packet at sequence 0. Its 25-byte
MSU-MR body contains a zero timestamp and MCUN, instrument marker `fff0`, JPEG
quality 50, and fourteen `001010` bit strings (zero DC difference followed by
end-of-block), padded with zero bits. A CCSDS APID 2047 idle packet fills the
remaining payload.

RS encoding uses conventional-basis GF(256), primitive polynomial `0x187`,
roots `alpha^((112+i)*11)` for `i = 0..31`, and systematic polynomial division.
No source code from the runtime decoder is imported by the fixture generator.
The UI test adds CCSDS randomization, sync `1acffc1d`, NRZ-M, inverted K=7
`(171,133)` convolutional outputs, 8 samples per symbol, and signed I/Q ±40
with the Q branch delayed by half a symbol.

Expected output: one APID 65 image, 1568×8, whose first 112 columns are 128
and remaining missing-segment pixels are zero. The worker test checks all
pixels, all four RS codewords, byte progress, and the exported PNG.
