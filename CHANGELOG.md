# Changelog

An EtherCAT master written in Rust.

<!-- next-header -->

## [Unreleased] - ReleaseDate

### Added

- [#47] Add the ability to read/write registers/SDOs from grouped slave devices, with the methods
  `SlaveRef::register_read`, `SlaveRef::register_write`, `SlaveRef::sdo_read` and
  `SlaveRef::sdo_write`.
- [#30] Added `Copy`, `Clone`, `PartialEq` and `Eq` implementations to `Error` and `PduError`.
- [#1] Added `SlaveGroup::len` and `SlaveGroup::is_empty` methods.
- [#29] Implement `Display` for `Error`, `PduError`, `MailboxError`, `EepromError`,
  `VisibleStringError` and `PduValidationError`
- **(breaking)** [#31] Added a `ClientConfig` argument to `Client::new` to allow configuration of
  various EtherCrab behaviours.

### Removed

- **(breaking)** [#45] The `SlaveGroupContainer` trait is no longer needed and has been removed.

### Changed

- [#47] Slave `sdo_read` and `sdo_write` methods no longer require the use of `SubIndex`. For single
  accesses, a raw `u8` can be passed instead for cleaner configuration code.
- **(breaking)** [#47] `SlaveGroup::slave` and `SlaveGroup::iter` (was `slaves`) now requires the
  passing of a `Client` reference when called.
- **(breaking)** [#47] `SlaveGroup::slaves` is renamed to `SlaveGroup::iter`
- **(breaking)** [#47] Grouped slaves that were previously represented as `GroupSlave`s are now
  represented as `SlaveRef<'_, SlavePdi<'_>>` instead. `GroupSlave` is removed.
- **(breaking)** [#47] The `io()`, `inputs()` and `outputs()` methods on grouped slaves have been
  renamed to `io_raw()`, `inputs_raw()` and `outputs_raw()` respecitively.
- **(breaking)** [#47] The `Slave.name` and `Slave.identity` fields have been replaced with methods
  of the same name.
- **(breaking)** [#45] The grouping closure passed to `Client::init` now requires a
  `&dyn SlaveGroupHandle` to be returned. This is a sealed trait only implemented for `SlaveGroup`s
  and allows some internal refactors by erasing the const generics from `SlaveGroup`.
- **(breaking)** [#32] To mitigate some internal issues, `PduStorage` now requires `N` (the number
  of storage elements) to be a power of two.
- **(breaking)** [#33] `send_frames_blocking` is removed. It is replaced with
  `PduTx::next_sendable_frame` which can be used to send any available frames in a loop until it
  returns `None`.
- **(breaking)** [#30] Removed `PduError::Encode` variant.
- **(breaking)** [#25] Renamed `pdu_rx` to `receive_frame` to mirror `send_frames_blocking`.
- **(breaking)** [#20] Changed the way the client, tx and rx instances are initialised to only allow
  one TX and RX to exist.

  Before

  ```rust
  static PDU_STORAGE: PduStorage<MAX_FRAMES, MAX_PDU_DATA> = PduStorage::new();
  static PDU_LOOP: PduLoop = PduLoop::new(PDU_STORAGE.as_ref());

  async fn main_app(ex: &LocalExecutor<'static>) -> Result<(), Error> {
      let client = Arc::new(Client::new(&PDU_LOOP, Timeouts::default()));

      ex.spawn(tx_rx_task(INTERFACE, &client).unwrap()).detach();
  }
  ```

  After

  ```rust
  static PDU_STORAGE: PduStorage<MAX_FRAMES, MAX_PDU_DATA> = PduStorage::new();

  async fn main_app(ex: &LocalExecutor<'static>) -> Result<(), Error> {
      let (tx, rx, pdu_loop) = PDU_STORAGE.try_split().expect("can only split once");

      let client = Arc::new(Client::new(pdu_loop, Timeouts::default()));

      ex.spawn(tx_rx_task(INTERFACE, tx, rx).unwrap()).detach();
  }
  ```

- **(breaking)** [#16] Remove `TIMER`/`TIMEOUT` generic parameter. `std` environments will now use
  the timer provided by `smol` (`async-io`). `no_std` environments will use `embassy-time`.
- **(breaking)** [#9] Rename the fields of some variants in `ethercrab::error::Error` to make them
  less confusing.
- **(breaking)** [#2] Rename `slave_group::Configurator` to `SlaveGroupRef`.
- **(breaking)** [#1] `SlaveGroup::slaves` now returns an iterator over each slave with IO in the
  group, instead of a plain slave.

### Fixed

- [#28] Fix abort code parsing for expedited SDO responses.
- [#26] :tada: EtherCrab now works on stable Rust. The MSRV is 1.68.
- [#23] Strip trailing null bytes (`\0`) in strings read from SII
- [#14] Fixed various overflow/arithmetic bugs in distributed clock time calculations and PDI
  configuration
- [#6] Fixed topology detection not stopping at first upstream fork when there is a slave device
  before the fork.
- [#6] Internal bugfixes to topology discovery code.
- [#2] Fixed multiple group PDI mapping calculation during initialisation.

## [0.1.0] - 2023-01-02

### Added

- Initial release

<!-- next-url -->

[#2]: https://github.com/ethercrab-rs/ethercrab/pull/2
[#1]: https://github.com/ethercrab-rs/ethercrab/pull/1
[#6]: https://github.com/ethercrab-rs/ethercrab/pull/6
[#9]: https://github.com/ethercrab-rs/ethercrab/pull/9
[#14]: https://github.com/ethercrab-rs/ethercrab/pull/14
[#16]: https://github.com/ethercrab-rs/ethercrab/pull/16
[#23]: https://github.com/ethercrab-rs/ethercrab/pull/23
[#20]: https://github.com/ethercrab-rs/ethercrab/pull/20
[#25]: https://github.com/ethercrab-rs/ethercrab/pull/25
[#26]: https://github.com/ethercrab-rs/ethercrab/pull/26
[#28]: https://github.com/ethercrab-rs/ethercrab/pull/28
[#29]: https://github.com/ethercrab-rs/ethercrab/pull/29
[#30]: https://github.com/ethercrab-rs/ethercrab/pull/30
[#31]: https://github.com/ethercrab-rs/ethercrab/pull/31
[#32]: https://github.com/ethercrab-rs/ethercrab/pull/32
[#33]: https://github.com/ethercrab-rs/ethercrab/pull/33
[#45]: https://github.com/ethercrab-rs/ethercrab/pull/45
[#47]: https://github.com/ethercrab-rs/ethercrab/pull/47
[unreleased]: https://github.com/ethercrab-rs/ethercrab/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/ethercrab-rs/ethercrab/compare/fb37346...v0.1.0
