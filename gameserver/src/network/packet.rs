use super::super::error::{AppError, PacketError};
use byteorder::{BE, ByteOrder};
use sonettobuf::prost::Message;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CompatibilityCommand(CompatibilityCommandKind);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CompatibilityCommandKind {
    CurrencyExchangeSameCurrency,
    TowerComposeGetInfo,
    PartyMatchPartyServerList,
}

impl CompatibilityCommand {
    const ALL: [Self; 3] = [
        Self(CompatibilityCommandKind::CurrencyExchangeSameCurrency),
        Self(CompatibilityCommandKind::TowerComposeGetInfo),
        Self(CompatibilityCommandKind::PartyMatchPartyServerList),
    ];

    pub(super) fn from_raw_id(raw_id: i16) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|command| command.raw_id() == raw_id)
    }

    pub(crate) const fn raw_id(self) -> i16 {
        match self.0 {
            CompatibilityCommandKind::CurrencyExchangeSameCurrency => 20144,
            CompatibilityCommandKind::TowerComposeGetInfo => 13540,
            CompatibilityCommandKind::PartyMatchPartyServerList => -16527,
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self.0 {
            CompatibilityCommandKind::CurrencyExchangeSameCurrency => {
                "Currency.ExchangeSameCurrencyRequest"
            }
            CompatibilityCommandKind::TowerComposeGetInfo => {
                "TowerCompose.TowerComposeGetInfoRequest"
            }
            CompatibilityCommandKind::PartyMatchPartyServerList => {
                "PartyMatch.PartyServerListRequest"
            }
        }
    }

    pub(super) const fn success_body(self) -> &'static [u8] {
        match self.0 {
            CompatibilityCommandKind::CurrencyExchangeSameCurrency => &[],
            CompatibilityCommandKind::TowerComposeGetInfo => &[0x0A, 0x00],
            CompatibilityCommandKind::PartyMatchPartyServerList => &[],
        }
    }
}

#[derive(Debug)]
pub struct ServerPacket {
    pub cmd_id: i16,
    pub result_code: u16,
    pub up_tag: u8,
    pub down_tag: u8,
    pub data: Vec<u8>,
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct ClientPacket {
    pub sequence: i32,
    pub cmd_id: i16,
    pub up_tag: u8,
    pub data: Vec<u8>,
}

#[allow(dead_code)]
impl ServerPacket {
    pub const PACKET_HEADER: usize = 10;

    pub fn encode(&self) -> Vec<u8> {
        let total_len = Self::PACKET_HEADER + self.data.len();
        let mut buffer = vec![0u8; total_len];

        BE::write_u32(&mut buffer[0..4], (total_len - 4) as u32);
        BE::write_i16(&mut buffer[4..6], self.cmd_id);
        BE::write_u16(&mut buffer[6..8], self.result_code);
        buffer[8] = self.up_tag;
        buffer[9] = self.down_tag;
        buffer[Self::PACKET_HEADER..].copy_from_slice(&self.data);

        buffer
    }

    pub fn decode(buffer: &[u8]) -> Result<Self, AppError> {
        if buffer.len() < Self::PACKET_HEADER {
            return Err(AppError::Packet(PacketError::LengthLessThanHeader(
                Self::PACKET_HEADER,
                buffer.len(),
            )));
        }

        let packet_size = BE::read_u32(&buffer[0..4]) as usize;
        if buffer.len() != packet_size + 4 {
            return Err(AppError::Packet(PacketError::LengthMismatch(
                packet_size + 4,
                buffer.len(),
            )));
        }

        let cmd_id = BE::read_i16(&buffer[4..6]);
        let result_code = BE::read_u16(&buffer[6..8]);
        let up_tag = buffer[8];
        let down_tag = buffer[9];
        let data = buffer[Self::PACKET_HEADER..].to_vec();

        Ok(Self {
            cmd_id,
            result_code,
            up_tag,
            down_tag,
            data,
        })
    }

    pub fn decode_message<T: Message + Default>(&self) -> Result<T, AppError> {
        T::decode(&*self.data)
            .map_err(|e| AppError::Packet(PacketError::ServerPacketDataDecodeFail(e)))
    }
}

impl ClientPacket {
    pub const PACKET_HEADER: usize = 11;

    #[allow(dead_code)]
    pub fn encode(&self) -> Vec<u8> {
        let total_len = Self::PACKET_HEADER + self.data.len();
        let mut buffer = vec![0u8; total_len];

        BE::write_i32(&mut buffer[0..4], (total_len - 4) as i32); // exclude the 4 bytes of length field
        BE::write_i32(&mut buffer[4..8], self.sequence);
        BE::write_i16(&mut buffer[8..10], self.cmd_id);
        buffer[10] = self.up_tag;
        buffer[Self::PACKET_HEADER..].copy_from_slice(&self.data);

        buffer
    }

    pub fn decode(buffer: &[u8]) -> Result<Self, AppError> {
        if buffer.len() < Self::PACKET_HEADER {
            return Err(AppError::Packet(PacketError::LengthLessThanHeader(
                Self::PACKET_HEADER,
                buffer.len(),
            )));
        }

        let packet_size = BE::read_i32(&buffer[0..4]) as usize;

        if buffer.len() != packet_size + 4 {
            return Err(AppError::Packet(PacketError::LengthMismatch(
                packet_size + 4,
                buffer.len(),
            )));
        }

        let sequence = BE::read_i32(&buffer[4..8]);
        let cmd_id = BE::read_i16(&buffer[8..10]);
        let up_tag = buffer[10];
        let data = buffer[Self::PACKET_HEADER..].to_vec();

        Ok(Self {
            sequence,
            cmd_id,
            up_tag,
            data,
        })
    }

    #[allow(dead_code)]
    pub fn decode_message<T: Message + Default>(&self) -> Result<T, AppError> {
        let data = &*self.data;
        let decoded = T::decode(data)
            .map_err(|e| AppError::Packet(PacketError::ClientPacketDataDecodeFail(e)))?;
        Ok(decoded)
    }
}

#[cfg(test)]
mod tests {
    use super::CompatibilityCommand;
    use sonettobuf::CmdId;

    #[test]
    fn compatibility_command_mapping_is_exact() {
        let currency_exchange = CompatibilityCommand::from_raw_id(20144).unwrap();
        let tower_compose = CompatibilityCommand::from_raw_id(13540).unwrap();
        let party_server_list = CompatibilityCommand::from_raw_id(-16527)
            .expect("missing PartyMatch.PartyServerListRequest compatibility command");

        assert_eq!(currency_exchange.raw_id(), 20144);
        assert_eq!(
            currency_exchange.name(),
            "Currency.ExchangeSameCurrencyRequest"
        );
        assert_eq!(tower_compose.raw_id(), 13540);
        assert_eq!(
            tower_compose.name(),
            "TowerCompose.TowerComposeGetInfoRequest"
        );
        assert_eq!(party_server_list.raw_id(), -16527);
        assert_eq!(
            party_server_list.name(),
            "PartyMatch.PartyServerListRequest"
        );
        assert_eq!(party_server_list.success_body(), &[] as &[u8]);
        assert!(CompatibilityCommand::from_raw_id(13539).is_none());
        assert!(CompatibilityCommand::from_raw_id(13541).is_none());
        assert!(CompatibilityCommand::from_raw_id(20143).is_none());
        assert!(CompatibilityCommand::from_raw_id(20145).is_none());
        assert!(CompatibilityCommand::from_raw_id(-16528).is_none());
        assert!(CompatibilityCommand::from_raw_id(-16526).is_none());
    }

    #[test]
    fn compatibility_commands_do_not_collide_with_registered_commands() {
        let compatibility_raw_ids = CompatibilityCommand::ALL
            .map(CompatibilityCommand::raw_id)
            .to_vec();
        assert_eq!(compatibility_raw_ids.as_slice(), &[20144, 13540, -16527]);

        for raw_id in [20144, 13540, -16527] {
            assert!(
                CmdId::try_from(raw_id as i32).is_err(),
                "compatibility raw command {} is now registered; remove its compatibility policy",
                raw_id
            );
        }
    }
}
