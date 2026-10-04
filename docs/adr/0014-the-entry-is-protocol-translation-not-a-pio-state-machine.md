# 0014. The entry is protocol translation, not a PIO state machine

- Status: Accepted
- Date: 2026-10-04
- Applies to: `ferrite-lithic`

## Context

Tiny Tapeout IHP 26b already contains multiple programmable-I/O cores on this exact
node: AstraPIO (`tt_um_fabien_pio`), nanoPIO (`tt_um_catalinlazar_nanopio`),
`tt_um_mini_kraken` and SEQ8 (`tt_um_jet_seq8b`). A fourth PIO cannot place.

The competition also explicitly rejects fixed logic: "The goal isn't to put a UART
block, an SPI block, and an I2C block on one die and call it done."

## Decision

The submission is a general-purpose CPU whose **firmware** implements a production
event-streaming protocol stack, starting from Loams' own contract:
`loams-stream-grpc`'s `StreamService`, whose comment already reads "used by
protocol adapters through Dapr invocation", including the CloudEvents
`application/cloudevents-batch+json` path annotated as a Dapr pass-through.

The JSON batch path is the target because it avoids both protobuf varints and
HTTP/2 HPACK, which is what makes it fit 11-26 KB of program memory.

## Consequences

We deliberately do not compete on the state-machine core. Differentiation is the
protocol stack running in firmware plus verification methodology, which is what the
competition says it rewards. Baseline UART, SPI and I2C still come first because the
post recommends that order and they prove the ISA generalises.
