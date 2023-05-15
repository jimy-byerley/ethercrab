use super::HookFn;
use crate::{
    error::Error,
    pdi::PdiOffset,
    register::RegisterAddress,
    slave::{
        configurator::{PdoDirection, SlaveConfigurator},
        slave_client::SlaveClient,
        Slave,
    },
    Client, SlaveGroup,
    slave_group::offset_helper::OffsetHelper
};
use core::{cell::UnsafeCell, time::Duration};

// use tokio::{
//     task::{self, JoinHandle, spawn},
//     runtime::Builder
// };
use futures::future::try_join_all;

#[derive(Debug)]
struct GroupInnerRef<'a> {
    slaves: &'a mut [Slave],
    /// The number of bytes at the beginning of the PDI reserved for slave inputs.
    read_pdi_len: &'a mut usize,
    /// The total length (I and O) of the PDI for this group.
    pdi_len: &'a mut usize,
    start_address: &'a mut u32,
    /// Expected working counter when performing a read/write to all slaves in this group.
    ///
    /// This should be equivalent to `(slaves with inputs) + (2 * slaves with outputs)`.
    group_working_counter: &'a mut u16,
}

/// A reference to a [`SlaveGroup`](crate::SlaveGroup) returned by the closure passed to
/// [`Client::init`](crate::Client::init).
pub struct SlaveGroupRef<'a> {
    max_pdi_len: usize,
    preop_safeop_hook: &'a Option<HookFn>,
    inner: UnsafeCell<GroupInnerRef<'a>>,
}

impl<'a> SlaveGroupRef<'a> {
    pub(in crate::slave_group) fn new<const MAX_SLAVES: usize, const MAX_PDI: usize>(
        group: &'a SlaveGroup<MAX_SLAVES, MAX_PDI>,
    ) -> Self {
        Self {
            max_pdi_len: MAX_PDI,
            preop_safeop_hook: &group.preop_safeop_hook,
            inner: {
                let inner = unsafe { &mut *group.inner.get() };

                UnsafeCell::new(GroupInnerRef {
                    slaves: &mut inner.slaves,
                    read_pdi_len: &mut inner.read_pdi_len,
                    pdi_len: &mut inner.pdi_len,
                    start_address: &mut inner.start_address,
                    group_working_counter: &mut inner.group_working_counter,
                })
            },
        }
    }

    #[deprecated="Replace by the function initialize_from_eeprom"]
    pub(crate) async unsafe fn configure_from_eeprom<'sto>(
        &self,
        // We need to start this group's PDI after that of the previous group. That offset is passed in via `start_offset`.
        mut global_offset: PdiOffset,
        client: &'sto Client<'sto>,
    ) -> Result<PdiOffset, Error> {
        let inner = unsafe { &mut *self.inner.get() };

        log::debug!(
            "Going to configure group with {} slave(s), starting PDI offset {:#08x}",
            inner.slaves.len(),
            global_offset.start_address
        );

        // Set the starting position in the PDI for this group's segment
        *inner.start_address = global_offset.start_address;

        // Configure master read PDI mappings in the first section of the PDI
        for slave in inner.slaves.iter_mut() {
            let mut slave_config = SlaveConfigurator::new(client, slave);

            // TODO: Split `SlaveGroupRef::configure_from_eeprom` so we can put all slaves into
            // SAFE-OP without waiting, then wait globally for all slaves to reach that state.
            // Currently startup time is extremely slow. NOTE: This method requests and waits for
            // the slave to enter PRE-OP
            slave_config.configure_mailboxes().await?;

            if let Some(hook) = self.preop_safeop_hook {
                let conf = slave_config.as_ref();

                let fut = (hook)(&conf);

                fut.await?;
            }

            // We're in PRE-OP at this point
            global_offset = slave_config
                .configure_fmmus(
                    global_offset,
                    *inner.start_address,
                    PdoDirection::MasterRead,
                )
                .await?;
        }

        *inner.read_pdi_len = (global_offset.start_address - *inner.start_address) as usize;

        log::debug!("Slave mailboxes configured and init hooks called");

        // We configured all read PDI mappings as a contiguous block in the previous loop. Now we'll
        // configure the write mappings in a separate loop. This means we have IIIIOOOO instead of
        // IOIOIO.
        for (_i, slave) in inner.slaves.iter_mut().enumerate() {
            let addr = slave.configured_address;
            let name = slave.name.clone();

            let mut slave_config = SlaveConfigurator::new(client, slave);

            // Still in PRE-OP
            global_offset = slave_config
                .configure_fmmus(
                    global_offset,
                    *inner.start_address,
                    PdoDirection::MasterWrite,
                )
                .await?;

            // FIXME: Just first slave or all slaves?
            // if name == "EL2004" {
            // if i == 0 {
            if false {
                log::info!("Slave {:#06x} {} DC", addr, name);
                let sl = SlaveClient::new(client, addr);

                // TODO: Pass in as config
                let cycle_time = Duration::from_millis(2).as_nanos() as u32;

                // Disable sync signals
                sl.write(RegisterAddress::DcSyncActive, 0x00u8, "disable sync")
                    .await?;

                let local_time: u32 = sl.read(RegisterAddress::DcSystemTime, "local time").await?;

                // TODO: Pass in as config
                // let startup_delay = Duration::from_millis(100).as_nanos() as u32;
                let startup_delay = 0;

                // TODO: Pass in as config
                let start_time = local_time + cycle_time + startup_delay;

                sl.write(
                    RegisterAddress::DcSyncStartTime,
                    start_time,
                    "sync start time",
                )
                .await?;

                sl.write(
                    RegisterAddress::DcSync0CycleTime,
                    cycle_time,
                    "sync cycle time",
                )
                .await?;

                // Enable cyclic operation (0th bit) and sync0 signal (1st bit)
                sl.write(RegisterAddress::DcSyncActive, 0b11u8, "enable sync0")
                    .await?;
            }

            // We're done configuring FMMUs, etc, now we can request this slave go into SAFE-OP
            slave_config.request_safe_op_nowait().await?;

            // We have both inputs and outputs at this stage, so can correctly calculate the group
            // WKC.
            *inner.group_working_counter += slave.config.io.working_counter_sum();
        }

        log::debug!("Slave FMMUs configured for group. Able to move to SAFE-OP");

        let pdi_len = (global_offset.start_address - *inner.start_address) as usize;

        log::debug!(
            "Group PDI length: start {}, {} total bytes ({} input bytes)",
            inner.start_address,
            pdi_len,
            *inner.read_pdi_len
        );

        if pdi_len > self.max_pdi_len {
            Err(Error::PdiTooLong {
                max_length: self.max_pdi_len,
                desired_length: pdi_len,
            })
        } else {
            *inner.pdi_len = pdi_len;

            Ok(global_offset)
        }
    }


    /// Configure slave group from eeprom value. This method is a part of configure_from_eeprom function
    /// Init all slave and set synchronization parameters
    pub(crate) async fn initialize_from_eeprom<'sto, const MAX_SLAVES: usize>(&self, global_offset: PdiOffset, client: &'sto Client<'sto>) -> Result<PdiOffset, Error> {

        let inner: &mut GroupInnerRef = unsafe { &mut *self.inner.get() };

        log::debug!("Going to configure group with {} slave(s), starting PDI offset {:#08x}", inner.slaves.len(), global_offset.start_address);

        // Set the starting position in the PDI for this group's segment
        *inner.start_address = global_offset.start_address;
        let mut offset_helper = OffsetHelper::new(global_offset, PdoDirection::MasterRead);

        let mut tasks = heapless::Vec::<_,MAX_SLAVES>::new();
        for slave in inner.slaves.iter_mut() {
            tasks.push(self.initialize_slave_config(slave, client));
        }

        // We're in PRE-OP at this point
        for result in try_join_all(tasks).await.unwrap().iter_mut() {
            _ = offset_helper.configure_fmmus(result).await?;
        }

        // Configure master read PDI mappings in the first section of the PDI
        log::debug!("{} Slave mailboxes configured", offset_helper.calling_cnt());
        *inner.read_pdi_len = offset_helper.finish();

        log::debug!("Slave mailboxes configured and init hooks called");

        let mut offset_helper = OffsetHelper::new(global_offset, PdoDirection::MasterWrite);
        for slave in inner.slaves.iter_mut() {

            let addr : u16 = slave.configured_address.clone();
            let name : heapless::String<64> = slave.name.clone();
            let mut slave_config_n = SlaveConfigurator::new(client, slave);

            // Still in PRE-OP
            offset_helper.configure_fmmus(&mut slave_config_n).await?;

            if false { self.set_synchronization(addr, name, client, None, None).await?; }

            // We're done configuring FMMUs, etc, now we can request this slave go into SAFE-OP
            slave_config_n.request_safe_op_nowait().await?;

            // We have both inputs and outputs at this stage, so can correctly calculate the group WKC.
            *inner.group_working_counter += slave.config.io.working_counter_sum();
        }

        log::debug!("Slave FMMUs configured for group. Able to move to SAFE-OP");

        let pdi_len = offset_helper.finish();

        log::debug!( "Group PDI length: start {}, {} total bytes ({} input bytes)", inner.start_address, pdi_len, *inner.read_pdi_len );

        if pdi_len > self.max_pdi_len {
            Err(Error::PdiTooLong { max_length: self.max_pdi_len, desired_length: pdi_len })
        } else {
            *inner.pdi_len = pdi_len;
            Ok(global_offset)
        }
    }

    /// Initialize a slave configutor and execute a preop hook on it
    async fn initialize_slave_config<'sto>(&self,  slave:  &'sto mut Slave, client: &'sto Client<'sto>) -> Result<SlaveConfigurator<'sto>, Error> {
        let mut slave_config: SlaveConfigurator = SlaveConfigurator::new(client, slave);
        slave_config.configure_mailboxes().await.unwrap(); //await

        // Use ???
        // if let Some(hook) = self.preop_safeop_hook {
        //     let conf = slave_config.as_ref();
        //     let fut = (hook)(&conf);

        //     fut.await.unwrap();
        // }

        return Ok(slave_config);
    }

    /// Configure time for slave client
    /// cycle_t Time in ms associate to cyle of TxRx? - If 'Option::None' use 2ms as default
    /// startup_t Delay in ms assocation to the begining of TxRx? - If 'Option::None' use 100ms as default
    async fn set_synchronization<'sto>(&self, addr : u16, name : heapless::String<64>, client : &'sto Client<'sto>,
        startup_t : Option<u64>, cycle_t : Option<u64>) -> Result<(), Error> {

        // FIXME: Just first slave or all slaves? if name == "EL2004" { // if i == 0 {
        log::info!("Slave {:#06x} {} DC", addr, name);

        let sl = SlaveClient::new(client, addr);

        let cycle_time = Duration::from_millis(cycle_t.unwrap_or(2 )).as_nanos() as u32;
        let startup_delay = Duration::from_millis(startup_t.unwrap_or(100)).as_nanos() as u32;

        // Disable sync signals
        sl.write(RegisterAddress::DcSyncActive, 0x00u8, "disable sync").await?;

        let local_time: u32 = sl.read(RegisterAddress::DcSystemTime, "local time").await?;
        let start_time = local_time + cycle_time + startup_delay;

        sl.write(RegisterAddress::DcSyncStartTime, start_time, "sync start time").await?;
        sl.write(RegisterAddress::DcSync0CycleTime, cycle_time, "sync cycle time").await?;

        // Enable cyclic operation (0th bit) and sync0 signal (1st bit)
        sl.write(RegisterAddress::DcSyncActive, 0b11u8, "enable sync0").await?;

        return Ok(());
    }
}
