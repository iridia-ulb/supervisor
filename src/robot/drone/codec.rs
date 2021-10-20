use std::marker::PhantomData;
use bytes::{Buf, BufMut};
use crc_any::CRCu16;
use mavlink::{MavHeader, MavlinkVersion};
use tokio_util::codec::{Encoder, Decoder};

pub struct MavMessageCodec<M> {
    _phantom: PhantomData<M>,
}

impl<M: mavlink::Message> MavMessageCodec<M> {
    pub fn new() -> MavMessageCodec<M> {
        MavMessageCodec { _phantom: PhantomData {}}
    }
}

impl<M: mavlink::Message> Encoder<(mavlink::MavHeader, M)> for MavMessageCodec<M> {
    type Error = mavlink::error::MessageWriteError;
    
    fn encode(&mut self, message: (mavlink::MavHeader, M), dst: &mut bytes::BytesMut) -> Result<(), Self::Error> {        
        let msgid = message.1.message_id();
        let payload = message.1.ser();
        let header = &[
            mavlink::MAV_STX_V2,
            payload.len() as u8,
            0, //incompat_flags
            0, //compat_flags
            message.0.sequence,
            message.0.system_id,
            message.0.component_id,
            (msgid & 0x0000FF) as u8,
            ((msgid & 0x00FF00) >> 8) as u8,
            ((msgid & 0xFF0000) >> 16) as u8,
        ];
        let mut crc_calc = CRCu16::crc16mcrf4cc();
        crc_calc.digest(&header[1..]);
        crc_calc.digest(&payload[..]);
        crc_calc.digest(&[M::extra_crc(msgid)]);
        dst.put_slice(header);
        dst.put_slice(&payload[..]);
        dst.put_slice(&crc_calc.get_crc().to_le_bytes());
        Ok(())
    }
}

impl<M: mavlink::Message> Decoder for MavMessageCodec<M> {
    type Item = (mavlink::MavHeader, M);
    type Error = mavlink::error::MessageReadError;

    fn decode(&mut self, src: &mut bytes::BytesMut) -> std::result::Result<Option<Self::Item>, Self::Error> {
        match src.iter().position(|&byte| byte == mavlink::MAV_STX_V2) {
            Some(index) => {
                /* discard everything up to but excluding MAV_STX_V2 */
                src.advance(index);
                let payload_len = match src.get(1) {
                    Some(&len) => len as usize,
                    None => return Ok(None)
                };
                let has_signature = match src.get(2) {
                    Some(flags) => flags & 0x01 == 0x01, // MAVLINK_IFLAG_SIGNED
                    None => return Ok(None)
                };
                let mut message_len = 12 + payload_len;
                if has_signature {
                    message_len += 13;
                };
                if src.remaining() >= message_len {
                    /* skip over STX */
                    src.advance(1);
                    let payload_len = src.get_u8() as usize;
                    let incompat_flags = src.get_u8();
                    let compat_flags = src.get_u8();
                    let seq = src.get_u8();
                    let sysid = src.get_u8();
                    let compid = src.get_u8();
                    let mut msgid_buf = [0; 4];
                    msgid_buf[0] = src.get_u8();
                    msgid_buf[1] = src.get_u8();
                    msgid_buf[2] = src.get_u8();

                    let header_buf = &[
                        payload_len as u8,
                        incompat_flags,
                        compat_flags,
                        seq,
                        sysid,
                        compid,
                        msgid_buf[0],
                        msgid_buf[1],
                        msgid_buf[2],
                    ];
                    let msgid: u32 = u32::from_le_bytes(msgid_buf);
                    let payload = src.split_to(payload_len);
                    let crc = src.get_u16_le();
                    if has_signature { 
                        src.advance(13);
                    }
                    let mut crc_calc = CRCu16::crc16mcrf4cc();
                    crc_calc.digest(&header_buf[..]);
                    crc_calc.digest(&payload[..]);
                    let extra_crc = M::extra_crc(msgid);
            
                    crc_calc.digest(&[extra_crc]);
                    let recvd_crc = crc_calc.get_crc();
                    if recvd_crc == crc {
                        /* hack: we should have a CRC error here */
                        M::parse(MavlinkVersion::V2, msgid, &payload[..])
                            .map(|msg| Some((MavHeader {
                                sequence: seq,
                                system_id: sysid,
                                component_id: compid,
                            }, msg))
                        )
                        .map_err(|err| err.into())
                    }
                    else {
                        /* CRC check failed, skip this message */
                        Ok(None)
                    }
                }
                else {
                    Ok(None)
                }
            }
            None => Ok(None)
        }
    }
}
