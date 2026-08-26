use crate::obs::{Data, DataArray, OwnedData};
use std::ffi::CStr;

const KEY_DEVICES: &CStr = c"known_devices";
const KEY_SERIAL: &CStr = c"serial";
const KEY_NAME: &CStr = c"name";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Device {
    pub serial: String,
    pub name: String,
}

impl Device {
    pub fn label(&self) -> String {
        match self.name.is_empty() {
            true => self.serial.clone(),
            false => format!("{} ({})", self.name, self.serial),
        }
    }
}

#[derive(Default)]
pub struct Devices(Vec<Device>);

impl Devices {
    pub fn all(&self) -> &[Device] {
        &self.0
    }

    pub fn remember(&mut self, serial: &str, name: &str) {
        if serial.is_empty() {
            return;
        }
        match self.0.iter_mut().find(|device| device.serial == serial) {
            Some(_) if name.is_empty() => (),
            Some(device) => device.name = name.to_string(),
            None => self.0.push(Device {
                serial: serial.to_string(),
                name: name.to_string(),
            }),
        }
    }

    pub fn load(&mut self, settings: &Data) {
        let devices = settings.array(KEY_DEVICES);
        self.0 = (0..devices.len())
            .filter_map(|index| {
                let device = devices.item(index)?;
                let serial = device.string(KEY_SERIAL);
                (!serial.is_empty()).then(|| Device {
                    serial,
                    name: device.string(KEY_NAME),
                })
            })
            .collect();
    }

    pub fn save(&self, settings: &Data) {
        let devices = DataArray::new();
        for device in &self.0 {
            let item = OwnedData::new();
            item.set_string(KEY_SERIAL, &device.serial);
            item.set_string(KEY_NAME, &device.name);
            devices.push(&item);
        }
        settings.set_array(KEY_DEVICES, &devices);
    }
}
