# PEX codec provenance

The initial PEX model and codec in this crate were adapted from `papyrus-utility-pex` by the same project owner, at revision `4898fb8fea9e6fb4ed71ebd9ab4b21a93c19d442` of `papyrus-utility` (GPL-3.0). The adapted files are `binary.rs`, `dump.rs`, `errors.rs`, `model.rs`, `opcode.rs`, `reader.rs`, `validation.rs`, and `writer.rs`. They are used under the Folio repository's GPL-3.0 license with the owner's explicit authorization.

The Folio crate separates PEX layout from compiler target capabilities and adds local validation and independent synthetic format tests. No game SDK, third-party script, or external binary asset is included.

The inherited reader allowed a Creation Kit object-size convention in which the stored length includes its own four-byte size field. Folio checks both conventions against the bytes actually consumed, including in multi-object files.
