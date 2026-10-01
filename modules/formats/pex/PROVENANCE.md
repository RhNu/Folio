# PEX codec provenance

The PEX model and codec were initially adapted from `papyrus-utility-pex`, part of `papyrus-utility`, by the same project owner. The source revision is `4898fb8fea9e6fb4ed71ebd9ab4b21a93c19d442`, licensed under GPL-3.0. The adapted files are `binary.rs`, `dump.rs`, `errors.rs`, `model.rs`, `opcode.rs`, `reader.rs`, `validation.rs`, and `writer.rs`. Folio uses them under its [GPL-3.0 license](../../../LICENSE) with the owner's explicit authorization.

The Folio crate separates PEX layout from compiler target capabilities and provides local validation and independent synthetic format tests. It includes no game SDK, third-party script, or external binary asset.

The reader supports a Creation Kit object-size convention inherited from the original codec: the stored length can include its own four-byte size field. Folio checks both length conventions against the bytes consumed, including in files containing multiple objects.
