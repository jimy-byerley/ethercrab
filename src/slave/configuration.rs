use super::{Slave, SlaveRef};
use crate::{
    coe::SubIndex,
    eeprom::types::{
        FmmuEx, FmmuUsage, MailboxProtocols, Pdo, SiiOwner, 
        SyncManager, SyncManagerEnable, SyncManagerType,
        PdoDirection,
    },
    error::{Error, Item},
    fmmu::Fmmu,
    pdi::PdiOffset,
    pdi::PdiSegment,
    register::RegisterAddress,
    slave::types::{Mailbox, MailboxConfig},
    slave_state::SlaveState,
    sync_manager_channel::SyncManagerChannel,
    sync_manager_channel::{self, SM_BASE_ADDRESS, SM_TYPE_ADDRESS},
};
use num_enum::FromPrimitive;
use packed_struct::PackedStruct;

/// Configuation from EEPROM methods.
impl<'a> SlaveRef<'a, &'a mut Slave> {
    /// First stage configuration (INIT -> PRE-OP).
    ///
    /// Continue configuration by calling [`configure_fmmus`](SlaveConfigurator::configure_fmmus)
    pub(crate) async fn configure_mailboxes(&mut self) -> Result<(), Error> {
        // Force EEPROM into master mode. Some slaves require PDI mode for INIT -> PRE-OP
        // transition. This is mentioned in ETG2010 p. 146 under "Eeprom/@AssignToPd". We'll reset
        // to master mode here, now that the transition is complete.
        self.set_eeprom_mode(SiiOwner::Master).await?;

        let sync_managers = self.eeprom_sync_managers().await?;

        // Mailboxes must be configured in INIT state
        self.configure_mailbox_sms(&sync_managers).await?;

        // Some slaves must be in PDI EEPROM mode to transition from INIT to PRE-OP. This is
        // mentioned in ETG2010 p. 146 under "Eeprom/@AssignToPd"
        self.set_eeprom_mode(SiiOwner::Pdi).await?;

        self.request_slave_state(SlaveState::PreOp).await?;

        self.set_eeprom_mode(SiiOwner::Master).await?;

        Ok(())
    }

    /// Second state configuration (PRE-OP -> SAFE-OP).
    ///
    /// PDOs must be configured in the PRE-OP state.
    pub(crate) async fn configure_fmmus(
        &mut self,
        mut global_offset: PdiOffset,
        group_start_address: u32,
        direction: PdoDirection,
    ) -> Result<PdiOffset, Error> {
        let sync_managers = self.eeprom_sync_managers().await?;
        let fmmu_usage = self.eeprom_fmmus().await?;
        let fmmu_sm_mappings = self.eeprom_fmmu_mappings().await?;

        let (state, _status_code) = self.status().await?;

        if state != SlaveState::PreOp {
            log::error!(
                "Slave {:#06x} is in invalid state {}. Expected {}",
                self.configured_address,
                state,
                SlaveState::PreOp
            );

            return Err(Error::InvalidState {
                expected: SlaveState::PreOp,
                actual: state,
                configured_address: self.configured_address,
            });
        }

        let segment = {
            let has_coe = self.state.config.mailbox.supported_protocols
                            .contains(MailboxProtocols::COE)
                        && self.state.config.mailbox.read
                            .map(|mbox| mbox.len > 0)
                            .unwrap_or(false);
            log::debug!(
                "Slave {:#06x} has CoE: {has_coe:?}",
                self.configured_address
            );
            
            let range = if has_coe {
                self.configure_pdos_coe(
                    &sync_managers,
                    &fmmu_usage,
                    direction,
                    &mut global_offset,
                    ).await?
            } else {
                let pdos = self.eeprom_pdos(direction).await?;
                log::trace!("Slave PDOs {:?} {:#?}", direction, &pdos);
                
                self.configure_pdos_eeprom(
                    &sync_managers,
                    &pdos,
                    &fmmu_sm_mappings,
                    &fmmu_usage,
                    direction,
                    &mut global_offset,
                    ).await?
            };
            PdiSegment {
                bytes: (range.bytes.start - group_start_address as usize)
                    ..(range.bytes.end - group_start_address as usize),
                .. range
            }
        };

        match direction {
            PdoDirection::MasterRead =>  {self.state.config.io.input  = segment},
            PdoDirection::MasterWrite => {self.state.config.io.output = segment},
        }

        log::debug!(
            "Slave {:#06x} PDI inputs: {:?} ({} bytes), outputs: {:?} ({} bytes)",
            self.configured_address,
            self.state.config.io.input,
            self.state.config.io.input.len(),
            self.state.config.io.output,
            self.state.config.io.output.len(),
        );

        Ok(global_offset)
    }
   
    pub(crate) async fn request_safe_op_nowait(&self) -> Result<(), Error> {
        // Restore EEPROM mode
        self.set_eeprom_mode(SiiOwner::Pdi).await?;

        self.request_slave_state_nowait(SlaveState::SafeOp).await?;

        Ok(())
    }

    async fn write_sm_config(
        &self,
        sync_manager_index: u8,
        sync_manager: &SyncManager,
        length_bytes: u16,
    ) -> Result<SyncManagerChannel, Error> {
        let sm_config = SyncManagerChannel {
            physical_start_address: sync_manager.start_addr,
            // Bit length, rounded up to the nearest byte
            length_bytes,
            control: sync_manager.control,
            status: Default::default(),
            enable: sync_manager_channel::Enable {
                enable: sync_manager.enable.contains(SyncManagerEnable::ENABLE),
                ..Default::default()
            },
        };

        self.write(
            RegisterAddress::sync_manager(sync_manager_index),
            sm_config.pack().unwrap(),
            "SM config",
        )
        .await?;

        log::debug!(
            "Slave {:#06x} SM{sync_manager_index}: {}",
            self.configured_address,
            sm_config
        );
        log::trace!("{:#?}", sm_config);

        Ok(sm_config)
    }

    /// Configure SM0 and SM1 for mailbox communication.
    async fn configure_mailbox_sms(&mut self, sync_managers: &[SyncManager]) -> Result<(), Error> {
        // Read default mailbox configuration from slave information area
        let mailbox_config = self.eeprom_mailbox_config().await?;

        log::trace!(
            "Slave {:#06x} Mailbox configuration: {:#?}",
            self.configured_address,
            mailbox_config
        );

        if !mailbox_config.has_mailbox() {
            log::trace!(
                "Slave {:#06x} has no valid mailbox configuration",
                self.configured_address
            );

            return Ok(());
        }

        let mut read_mailbox = None;
        let mut write_mailbox = None;

        for (sync_manager_index, sync_manager) in sync_managers.iter().enumerate() {
            let sync_manager_index = sync_manager_index as u8;

            // Mailboxes are configured in INIT state
            match sync_manager.usage_type {
                SyncManagerType::MailboxWrite => {
                    self.write_sm_config(
                        sync_manager_index,
                        sync_manager,
                        mailbox_config.slave_receive_size,
                    )
                    .await?;

                    write_mailbox = Some(Mailbox {
                        address: sync_manager.start_addr,
                        len: mailbox_config.slave_receive_size,
                        sync_manager: sync_manager_index,
                    });
                }
                SyncManagerType::MailboxRead => {
                    self.write_sm_config(
                        sync_manager_index,
                        sync_manager,
                        mailbox_config.slave_send_size,
                    )
                    .await?;

                    read_mailbox = Some(Mailbox {
                        address: sync_manager.start_addr,
                        len: mailbox_config.slave_receive_size,
                        sync_manager: sync_manager_index,
                    });
                }
                _ => continue,
            }
        }

        self.state.config.mailbox = MailboxConfig {
            read: read_mailbox,
            write: write_mailbox,
            supported_protocols: mailbox_config.supported_protocols,
        };

        Ok(())
    }

    /// Configure PDOs from CoE registers.
    async fn configure_pdos_coe(
        &self,
        sync_managers: &[SyncManager],
        fmmu_usage: &[FmmuUsage],
        direction: PdoDirection,
        gobal_offset: &mut PdiOffset,
    ) -> Result<PdiSegment, Error> {
        let (desired_sm_type, desired_fmmu_type) = direction.filter_terms();

        // ETG1000.6 Table 67 – CoE Communication Area
        let num_sms = self
            .sdo_read::<u8>(SM_TYPE_ADDRESS, SubIndex::Index(0))
            .await?;

        log::trace!("Found {num_sms} SMs from CoE");

        let start_offset = *gobal_offset;

        let mut total_bit_len = 0;

        // We must ignore the first two SM indices (SM0, SM1, sub-index 1 and 2, start at sub-index
        // 3) as these are used for mailbox communication.
        let sm_range = 3..=num_sms;
        // NOTE: This is a 1-based SDO sub-index
        for sm_mapping_sub_index in sm_range {
            let sm_type = self
                .sdo_read::<u8>(SM_TYPE_ADDRESS, SubIndex::Index(sm_mapping_sub_index))
                .await
                .map(SyncManagerType::from_primitive)?;

            // this sm does not fit our direction need
            if sm_type != desired_sm_type 
                {continue;}

            // zero-based index
            let sync_manager_index = sm_mapping_sub_index - 1;
            let sm_address = SM_BASE_ADDRESS + u16::from(sync_manager_index);

            let sync_manager = sync_managers
                    .get(usize::from(sync_manager_index))
                    .ok_or(Error::NotFound {
                        item: Item::SyncManager,
                        index: Some(usize::from(sync_manager_index)),
                    })?;

            // Total number of PDO assignments for this sync manager
            let num_sm_assignments = self.sdo_read::<u8>(sm_address, SubIndex::Index(0)).await?;

            log::trace!("SDO sync manager {sync_manager_index} (sub index #{sm_mapping_sub_index}) {sm_address:#06x} {sm_type:?}, sub indices: {num_sm_assignments}");

            let mut sm_bit_len = 0u16;

            for i in 1..=num_sm_assignments {
                let pdo = self.sdo_read::<u16>(sm_address, SubIndex::Index(i)).await?;
                let num_mappings = self.sdo_read::<u8>(pdo, SubIndex::Index(0)).await?;

                log::trace!("--> #{i} data: {pdo:#06x} ({num_mappings} mappings):");

                for i in 1..=num_mappings {
                    let mapping = self.sdo_read::<u32>(pdo, SubIndex::Index(i)).await?;

                    // Yes, big-endian. Makes life easier when mapping from debug prints to actual
                    // data fields.
                    let parts = mapping.to_be_bytes();

                    let index = u16::from_be_bytes(parts[0..=1].try_into().unwrap());
                    let sub_index = parts[2];
                    let mapping_bit_len = parts[3];

                    log::trace!(
                        "----> index {index:#06x}, sub index {sub_index}, bit length {mapping_bit_len}"
                    );

                    sm_bit_len += u16::from(mapping_bit_len);
                }
            }

            log::trace!(
                "----= total SM bit length {sm_bit_len} ({} bytes)",
                (sm_bit_len + 7) / 8
            );

            let sm_config = self
                .write_sm_config(sync_manager_index, sync_manager, (sm_bit_len + 7) / 8)
                .await?;

            if sm_bit_len > 0 {
                let fmmu_index = fmmu_usage
                    .iter()
                    .position(|usage| *usage == desired_fmmu_type)
                    .ok_or(Error::NotFound {
                        item: Item::Fmmu,
                        index: None,
                    })?;

                self.write_fmmu_config(
                    sm_bit_len,
                    fmmu_index,
                    gobal_offset,
                    desired_sm_type,
                    &sm_config,
                    ).await?;
            }

            total_bit_len += sm_bit_len;
        }

        Ok(PdiSegment {
            bit_len: total_bit_len.into(),
            bytes: start_offset.up_to(*gobal_offset),
        })
    }

    async fn write_fmmu_config(
        &self,
        sm_bit_len: u16,
        fmmu_index: usize,
        global_offset: &mut PdiOffset,
        desired_sm_type: SyncManagerType,
        sm_config: &SyncManagerChannel,
    ) -> Result<(), Error> {
        // Multiple SMs may use the same FMMU, so we'll read the existing config from the slave
        let mut fmmu_config = self
            .read::<[u8; 16]>(RegisterAddress::fmmu(fmmu_index as u8), "read FMMU config")
            .await
            .and_then(|raw| Fmmu::unpack(&raw).map_err(|_| Error::Internal))?;

        // We can use the enable flag as a sentinel for existing config because EtherCrab inits
        // FMMUs to all zeroes on startup.
        let fmmu_config = if fmmu_config.enable {
            fmmu_config.length_bytes += sm_config.length_bytes;

            fmmu_config
        } else {
            Fmmu {
                logical_start_address: global_offset.start_address,
                length_bytes: sm_config.length_bytes,
                // Mapping into PDI is byte-aligned until/if we support bit-oriented slaves
                logical_start_bit: 0,
                // Always byte-aligned
                logical_end_bit: 7,
                physical_start_address: sm_config.physical_start_address,
                physical_start_bit: 0x0,
                read_enable: desired_sm_type == SyncManagerType::ProcessDataRead,
                write_enable: desired_sm_type == SyncManagerType::ProcessDataWrite,
                enable: true,
            }
        };

        self.write(
            RegisterAddress::fmmu(fmmu_index as u8),
            fmmu_config.pack().unwrap(),
            "PDI FMMU",
        )
        .await?;
        log::debug!(
            "Slave {:#06x} FMMU{fmmu_index}: {}",
            self.configured_address,
            fmmu_config
        );
        log::trace!("{:#?}", fmmu_config);
        *global_offset = global_offset.increment_byte_aligned(sm_bit_len);
        Ok(())
    }

    /// Configure PDOs from EEPROM
    async fn configure_pdos_eeprom(
        &self,
        sync_managers: &[SyncManager],
        pdos: &[Pdo],
        fmmu_sm_mappings: &[FmmuEx],
        fmmu_usage: &[FmmuUsage],
        direction: PdoDirection,
        offset: &mut PdiOffset,
    ) -> Result<PdiSegment, Error> {
        let start_offset = *offset;
        let mut total_bit_len = 0;

        let (sm_type, fmmu_type) = direction.filter_terms();

        for (sync_manager_index, sync_manager) in sync_managers
            .iter()
            .enumerate()
            .filter(|(_idx, sm)| sm.usage_type == sm_type)
        {
            let sync_manager_index = sync_manager_index as u8;

            let bit_len = pdos
                .iter()
                .filter(|pdo| pdo.sync_manager == sync_manager_index)
                .map(|pdo| pdo.bit_len())
                .sum();

            total_bit_len += bit_len;

            // Look for FMMU index using FMMU_EX section in EEPROM. If it's empty, default
            // to looking through FMMU usage list and picking out the appropriate kind
            // (Inputs, Outputs)
            let fmmu_index = fmmu_sm_mappings
                .iter()
                .find(|fmmu| fmmu.sync_manager == sync_manager_index)
                .map(|fmmu| fmmu.sync_manager)
                .or_else(|| {
                    log::trace!("Could not find FMMU for PDO SM{sync_manager_index}");

                    fmmu_usage
                        .iter()
                        .position(|usage| *usage == fmmu_type)
                        .map(|idx| {
                            log::trace!("Using fallback FMMU FMMU{idx}");

                            idx as u8
                        })
                })
                .ok_or(Error::NotFound {
                    item: Item::Fmmu,
                    index: None,
                })?;

            let sm_config = self
                .write_sm_config(sync_manager_index, sync_manager, (bit_len + 7) / 8)
                .await?;

            self.write_fmmu_config(
                bit_len,
                usize::from(fmmu_index),
                offset,
                sm_type,
                &sm_config,
            )
            .await?;
        }

        Ok(PdiSegment {
            bit_len: total_bit_len.into(),
            bytes: start_offset.up_to(*offset),
        })
    }
}
