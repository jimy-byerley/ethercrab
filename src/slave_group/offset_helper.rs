use crate::{
    slave::configurator::{SlaveConfigurator, PdoDirection},
    pdi::PdiOffset,
    error::Error
};

// pub(crate) struct OffsetHelper<'a> {
pub(crate) struct OffsetHelper {
    _cur_offset : PdiOffset,
    _start_address : u32,
    _index : u32,
    _direction  : PdoDirection,
}


/// Helper class to configure slave fmumus and shift retains offset state
/// 1. Call 'new' fn to setup the helper
/// 2. Call n times 'configure_fmmus' with the current Pdo direction
/// 3. Call "finalize" to consume this struct and return the difference between current handling offset and the start offset
impl OffsetHelper{

    ///Ctor. Take an offset as initialisation and a PdoDirection used in each call of configuration_fmmus function
    ///Return self
    pub fn new(offset : PdiOffset, direction : PdoDirection) -> Self {
        return Self {
            _cur_offset : offset,
            _start_address : offset.start_address,
            _index : 0,
            _direction : direction
        };
    }

    ///Configure fmmus and update offset internaly handle
    /// slvc: The slave configurator that is endure fmmus configuration
    /// Return ()
    pub async fn configure_fmmus<'a,'b>(&mut self, slvc : &'a mut SlaveConfigurator<'b>) -> Result<(), Error> {
        self._cur_offset = slvc.configure_fmmus(self._cur_offset, self._start_address, self._direction).await?;
        self._index += 1;
        return Ok(());
    }


    /// Return the number of call to the configure_fmmus function
    pub fn calling_cnt(&self) -> u32 {
        return self._index;
    }

    //Comsume self and return Pdi length
    pub fn finish(self) -> usize {
        return (self._cur_offset.start_address - self._start_address) as usize;
    }
}
