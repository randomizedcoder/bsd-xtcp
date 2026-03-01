use crate::platform::macos_layout::*;
use crate::platform::CollectError;
use crate::record::{IpAddr, RawSocketRecord};

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
use crate::platform::CollectionResult;

#[cfg(any(target_os = "macos", target_os = "freebsd"))]
const PCBLIST_SYSCTL: &str = "net.inet.tcp.pcblist_n";
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
const MAX_RETRIES: u32 = 3;

/// Collect TCP socket records from the macOS kernel via pcblist_n.
#[cfg(any(target_os = "macos", target_os = "freebsd"))]
pub fn collect() -> Result<CollectionResult, CollectError> {
    use crate::sysctl;

    let start = std::time::Instant::now();
    let hz = sysctl::read_clock_hz()?;
    let (buf, generation) = sysctl::read_pcblist_validated(PCBLIST_SYSCTL, MAX_RETRIES)?;
    let records = parse_pcblist_n(&buf, hz)?;
    let duration = start.elapsed().as_nanos() as u64;

    Ok(CollectionResult {
        records,
        generation,
        collection_duration_ns: duration,
    })
}

// --- Byte-reading helpers ---

fn read_u8_at(buf: &[u8], offset: usize) -> Result<u8, CollectError> {
    buf.get(offset).copied().ok_or(CollectError::Truncated {
        offset,
        need: 1,
        have: buf.len(),
    })
}

fn read_u16_be_at(buf: &[u8], offset: usize) -> Result<u16, CollectError> {
    let end = offset + 2;
    if end > buf.len() {
        return Err(CollectError::Truncated {
            offset,
            need: 2,
            have: buf.len(),
        });
    }
    Ok(u16::from_be_bytes([buf[offset], buf[offset + 1]]))
}

fn read_i32_at(buf: &[u8], offset: usize) -> Result<i32, CollectError> {
    let end = offset + 4;
    if end > buf.len() {
        return Err(CollectError::Truncated {
            offset,
            need: 4,
            have: buf.len(),
        });
    }
    Ok(i32::from_ne_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ]))
}

fn read_u32_at(buf: &[u8], offset: usize) -> Result<u32, CollectError> {
    let end = offset + 4;
    if end > buf.len() {
        return Err(CollectError::Truncated {
            offset,
            need: 4,
            have: buf.len(),
        });
    }
    Ok(u32::from_ne_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ]))
}

fn read_u64_at(buf: &[u8], offset: usize) -> Result<u64, CollectError> {
    let end = offset + 8;
    if end > buf.len() {
        return Err(CollectError::Truncated {
            offset,
            need: 8,
            have: buf.len(),
        });
    }
    Ok(u64::from_ne_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
        buf[offset + 4],
        buf[offset + 5],
        buf[offset + 6],
        buf[offset + 7],
    ]))
}

/// Accumulates fields from tagged records for one connection.
#[derive(Default)]
struct ConnectionAccumulator {
    rec: RawSocketRecord,
    has_socket: bool,
}

impl ConnectionAccumulator {
    fn parse_xsocket_n(&mut self, body: &[u8]) -> Result<(), CollectError> {
        self.has_socket = true;
        if body.len() > XSOCKET_N_SO_PCB_OFFSET + 8 {
            self.rec.socket_id = Some(read_u64_at(body, XSOCKET_N_SO_PCB_OFFSET)?);
        }
        if body.len() > XSOCKET_N_SO_UID_OFFSET + 4 {
            self.rec.uid = Some(read_u32_at(body, XSOCKET_N_SO_UID_OFFSET)?);
        }
        if body.len() > XSOCKET_N_SO_LAST_PID_OFFSET + 4 {
            self.rec.pid = Some(read_i32_at(body, XSOCKET_N_SO_LAST_PID_OFFSET)?);
        }
        if body.len() > XSOCKET_N_SO_E_PID_OFFSET + 4 {
            self.rec.effective_pid = Some(read_i32_at(body, XSOCKET_N_SO_E_PID_OFFSET)?);
        }
        Ok(())
    }

    fn parse_rcvbuf(&mut self, body: &[u8]) -> Result<(), CollectError> {
        if body.len() > XSOCKBUF_N_CC_OFFSET + 4 {
            self.rec.rcv_buf_used = Some(read_u32_at(body, XSOCKBUF_N_CC_OFFSET)?);
        }
        if body.len() > XSOCKBUF_N_HIWAT_OFFSET + 4 {
            self.rec.rcv_buf_hiwat = Some(read_u32_at(body, XSOCKBUF_N_HIWAT_OFFSET)?);
        }
        Ok(())
    }

    fn parse_sndbuf(&mut self, body: &[u8]) -> Result<(), CollectError> {
        if body.len() > XSOCKBUF_N_CC_OFFSET + 4 {
            self.rec.snd_buf_used = Some(read_u32_at(body, XSOCKBUF_N_CC_OFFSET)?);
        }
        if body.len() > XSOCKBUF_N_HIWAT_OFFSET + 4 {
            self.rec.snd_buf_hiwat = Some(read_u32_at(body, XSOCKBUF_N_HIWAT_OFFSET)?);
        }
        Ok(())
    }

    fn parse_xinpcb_n(&mut self, body: &[u8]) -> Result<(), CollectError> {
        // Read vflag to determine IP version
        if body.len() <= XINPCB_N_INP_VFLAG_OFFSET {
            return Ok(());
        }
        let vflag = read_u8_at(body, XINPCB_N_INP_VFLAG_OFFSET)?;

        if vflag & INP_IPV6 != 0 {
            self.rec.ip_version = Some(6);
            // IPv6 addresses (16 bytes each)
            if body.len() > XINPCB_N_IN6P_LADDR_OFFSET + 16 {
                let mut addr = [0u8; 16];
                addr.copy_from_slice(
                    &body[XINPCB_N_IN6P_LADDR_OFFSET..XINPCB_N_IN6P_LADDR_OFFSET + 16],
                );
                self.rec.local_addr = Some(IpAddr::V6(addr));
            }
            if body.len() > XINPCB_N_IN6P_FADDR_OFFSET + 16 {
                let mut addr = [0u8; 16];
                addr.copy_from_slice(
                    &body[XINPCB_N_IN6P_FADDR_OFFSET..XINPCB_N_IN6P_FADDR_OFFSET + 16],
                );
                self.rec.remote_addr = Some(IpAddr::V6(addr));
            }
        } else if vflag & INP_IPV4 != 0 {
            self.rec.ip_version = Some(4);
            // IPv4 addresses (4 bytes each)
            if body.len() > XINPCB_N_INP_LADDR_OFFSET + 4 {
                let mut addr = [0u8; 4];
                addr.copy_from_slice(
                    &body[XINPCB_N_INP_LADDR_OFFSET..XINPCB_N_INP_LADDR_OFFSET + 4],
                );
                self.rec.local_addr = Some(IpAddr::V4(addr));
            }
            if body.len() > XINPCB_N_INP_FADDR_OFFSET + 4 {
                let mut addr = [0u8; 4];
                addr.copy_from_slice(
                    &body[XINPCB_N_INP_FADDR_OFFSET..XINPCB_N_INP_FADDR_OFFSET + 4],
                );
                self.rec.remote_addr = Some(IpAddr::V4(addr));
            }
        }

        // Ports (network byte order)
        if body.len() > XINPCB_N_INP_LPORT_OFFSET + 2 {
            self.rec.local_port = Some(read_u16_be_at(body, XINPCB_N_INP_LPORT_OFFSET)?);
        }
        if body.len() > XINPCB_N_INP_FPORT_OFFSET + 2 {
            self.rec.remote_port = Some(read_u16_be_at(body, XINPCB_N_INP_FPORT_OFFSET)?);
        }

        // inp_gencnt
        if body.len() > XINPCB_N_INP_GENCNT_OFFSET + 8 {
            self.rec.inp_gencnt = Some(read_u64_at(body, XINPCB_N_INP_GENCNT_OFFSET)?);
        }

        Ok(())
    }

    fn parse_xtcpcb_n(&mut self, body: &[u8], hz: i32) -> Result<(), CollectError> {
        if body.len() > XTCPCB_N_T_STATE_OFFSET + 4 {
            self.rec.state = Some(read_i32_at(body, XTCPCB_N_T_STATE_OFFSET)?);
        }
        if body.len() > XTCPCB_N_T_RXTSHIFT_OFFSET + 4 {
            self.rec.rxt_shift = Some(read_u32_at(body, XTCPCB_N_T_RXTSHIFT_OFFSET)?);
        }
        if body.len() > XTCPCB_N_T_FLAGS_OFFSET + 4 {
            self.rec.tcp_flags = Some(read_u32_at(body, XTCPCB_N_T_FLAGS_OFFSET)?);
        }
        if body.len() > XTCPCB_N_SND_CWND_OFFSET + 4 {
            self.rec.snd_cwnd = Some(read_u32_at(body, XTCPCB_N_SND_CWND_OFFSET)?);
        }
        if body.len() > XTCPCB_N_SND_SSTHRESH_OFFSET + 4 {
            self.rec.snd_ssthresh = Some(read_u32_at(body, XTCPCB_N_SND_SSTHRESH_OFFSET)?);
        }
        if body.len() > XTCPCB_N_T_MAXSEG_OFFSET + 4 {
            self.rec.maxseg = Some(read_u32_at(body, XTCPCB_N_T_MAXSEG_OFFSET)?);
        }

        // RTT: t_srtt is stored as (srtt << TCP_RTT_SHIFT) in ticks
        // Convert: ((t_srtt >> TCP_RTT_SHIFT) * 1_000_000) / hz
        if body.len() > XTCPCB_N_T_SRTT_OFFSET + 4 && hz > 0 {
            let raw_srtt = read_u32_at(body, XTCPCB_N_T_SRTT_OFFSET)?;
            let ticks = raw_srtt >> TCP_RTT_SHIFT;
            self.rec.rtt_us = Some((ticks as u64 * 1_000_000 / hz as u64) as u32);
        }
        if body.len() > XTCPCB_N_T_RTTVAR_OFFSET + 4 && hz > 0 {
            let raw_rttvar = read_u32_at(body, XTCPCB_N_T_RTTVAR_OFFSET)?;
            let ticks = raw_rttvar >> TCP_RTTVAR_SHIFT;
            self.rec.rttvar_us = Some((ticks as u64 * 1_000_000 / hz as u64) as u32);
        }

        // RTO (raw ticks)
        if body.len() > XTCPCB_N_T_RXTCUR_OFFSET + 4 && hz > 0 {
            let raw_rto = read_u32_at(body, XTCPCB_N_T_RXTCUR_OFFSET)?;
            self.rec.rto_us = Some((raw_rto as u64 * 1_000_000 / hz as u64) as u32);
        }

        // Sequence numbers
        if body.len() > XTCPCB_N_SND_NXT_OFFSET + 4 {
            self.rec.snd_nxt = Some(read_u32_at(body, XTCPCB_N_SND_NXT_OFFSET)?);
        }
        if body.len() > XTCPCB_N_SND_UNA_OFFSET + 4 {
            self.rec.snd_una = Some(read_u32_at(body, XTCPCB_N_SND_UNA_OFFSET)?);
        }
        if body.len() > XTCPCB_N_SND_MAX_OFFSET + 4 {
            self.rec.snd_max = Some(read_u32_at(body, XTCPCB_N_SND_MAX_OFFSET)?);
        }
        if body.len() > XTCPCB_N_RCV_NXT_OFFSET + 4 {
            self.rec.rcv_nxt = Some(read_u32_at(body, XTCPCB_N_RCV_NXT_OFFSET)?);
        }
        if body.len() > XTCPCB_N_RCV_ADV_OFFSET + 4 {
            self.rec.rcv_adv = Some(read_u32_at(body, XTCPCB_N_RCV_ADV_OFFSET)?);
        }

        // Windows
        if body.len() > XTCPCB_N_SND_WND_OFFSET + 4 {
            self.rec.snd_wnd = Some(read_u32_at(body, XTCPCB_N_SND_WND_OFFSET)?);
        }
        if body.len() > XTCPCB_N_RCV_WND_OFFSET + 4 {
            self.rec.rcv_wnd = Some(read_u32_at(body, XTCPCB_N_RCV_WND_OFFSET)?);
        }

        // Window scale (single bytes)
        if body.len() > XTCPCB_N_SND_WSCALE_OFFSET {
            self.rec.snd_wscale = Some(read_u8_at(body, XTCPCB_N_SND_WSCALE_OFFSET)? as u32);
        }
        if body.len() > XTCPCB_N_RCV_WSCALE_OFFSET {
            self.rec.rcv_wscale = Some(read_u8_at(body, XTCPCB_N_RCV_WSCALE_OFFSET)? as u32);
        }

        // Dupacks
        if body.len() > XTCPCB_N_T_DUPACKS_OFFSET + 4 {
            self.rec.dupacks = Some(read_u32_at(body, XTCPCB_N_T_DUPACKS_OFFSET)?);
        }

        // Start time
        if body.len() > XTCPCB_N_T_STARTTIME_OFFSET + 4 {
            self.rec.start_time_secs = Some(read_u32_at(body, XTCPCB_N_T_STARTTIME_OFFSET)?);
        }

        Ok(())
    }

    fn build(mut self) -> Option<RawSocketRecord> {
        if !self.has_socket {
            return None;
        }
        // Tag data source as macOS pcblist_n (DataSource::MacosPcblistN = 1)
        self.rec.sources = vec![1];
        Some(self.rec)
    }
}

/// Parse the raw pcblist_n buffer into a list of `RawSocketRecord`.
///
/// This is a pure function that can be tested on any platform with synthetic byte buffers.
pub fn parse_pcblist_n(buf: &[u8], hz: i32) -> Result<Vec<RawSocketRecord>, CollectError> {
    let mut records = Vec::new();

    if buf.len() < 4 {
        return Err(CollectError::Truncated {
            offset: 0,
            need: 4,
            have: buf.len(),
        });
    }

    // Skip the xinpgen header
    let header_len = read_u32_at(buf, XINPGEN_LEN_OFFSET)? as usize;
    if header_len == 0 || header_len > buf.len() {
        return Err(CollectError::Parse {
            offset: 0,
            message: format!("invalid header length: {header_len}"),
        });
    }

    let mut pos = roundup64(header_len as u32);
    let mut acc = ConnectionAccumulator::default();

    while pos + TAG_HEADER_SIZE <= buf.len() {
        let rec_len = read_u32_at(buf, pos + TAG_LEN_OFFSET)?;
        let rec_kind = read_u32_at(buf, pos + TAG_KIND_OFFSET)?;

        // A zero-length record or zero-kind marks the trailer
        if rec_len == 0 || rec_kind == 0 {
            break;
        }

        let rec_end = pos + rec_len as usize;
        if rec_end > buf.len() {
            return Err(CollectError::Truncated {
                offset: pos,
                need: rec_len as usize,
                have: buf.len() - pos,
            });
        }

        // Body starts after the 8-byte tag header
        let body = &buf[pos + TAG_HEADER_SIZE..rec_end];

        match rec_kind {
            XSO_SOCKET => {
                // New xsocket_n = new connection group. Emit previous if complete.
                if let Some(rec) = acc.build() {
                    records.push(rec);
                }
                acc = ConnectionAccumulator::default();
                acc.parse_xsocket_n(body)?;
            }
            XSO_RCVBUF => acc.parse_rcvbuf(body)?,
            XSO_SNDBUF => acc.parse_sndbuf(body)?,
            XSO_STATS => { /* stats record — skip for now */ }
            XSO_INPCB => acc.parse_xinpcb_n(body)?,
            XSO_TCPCB => acc.parse_xtcpcb_n(body, hz)?,
            _ => { /* Unknown kind — skip for forward compatibility */ }
        }

        // Advance cursor by the aligned record length
        pos += roundup64(rec_len);
    }

    // Emit the last accumulator
    if let Some(rec) = acc.build() {
        records.push(rec);
    }

    Ok(records)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a minimal xinpgen header.
    fn make_xinpgen(gen: u64) -> Vec<u8> {
        let mut buf = Vec::new();
        // xig_len: 24 bytes
        buf.extend_from_slice(&24u32.to_ne_bytes());
        // xig_count: 0
        buf.extend_from_slice(&0u32.to_ne_bytes());
        // xig_gen
        buf.extend_from_slice(&gen.to_ne_bytes());
        // xig_sogen
        buf.extend_from_slice(&0u64.to_ne_bytes());
        buf
    }

    /// Build a tagged record with the given kind and body.
    fn make_tagged_record(kind: u32, body: &[u8]) -> Vec<u8> {
        let total_len = (TAG_HEADER_SIZE + body.len()) as u32;
        let mut buf = Vec::new();
        buf.extend_from_slice(&total_len.to_ne_bytes());
        buf.extend_from_slice(&kind.to_ne_bytes());
        buf.extend_from_slice(body);
        // Pad to 8-byte alignment
        let aligned = roundup64(total_len);
        buf.resize(aligned, 0);
        buf
    }

    /// Build a trailer (xinpgen with matching gen).
    fn make_trailer(gen: u64) -> Vec<u8> {
        make_xinpgen(gen)
    }

    #[test]
    fn test_parse_empty_pcblist() {
        let gen = 42u64;
        let mut buf = make_xinpgen(gen);
        // Pad header to 8-byte alignment
        let header_aligned = roundup64(24);
        buf.resize(header_aligned, 0);
        // Add trailer (zero-kind record is implicit when we hit end of buffer)
        buf.extend_from_slice(&make_trailer(gen));

        // Should parse with 0 records (trailer has kind=0 implicitly via xig_count)
        // Actually trailer is an xinpgen, which starts with len but no tag header.
        // For this test, just verify empty parse doesn't panic.
        let result = parse_pcblist_n(&buf, 100);
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 0);
    }

    #[test]
    fn test_parse_single_connection() {
        let gen = 1u64;
        let mut buf = make_xinpgen(gen);
        let header_aligned = roundup64(24);
        buf.resize(header_aligned, 0);

        // xsocket_n body — needs to be large enough for pid/uid offsets
        let mut socket_body = vec![0u8; 80];
        // so_pcb at offset 8
        socket_body[8..16].copy_from_slice(&0xDEAD_BEEFu64.to_ne_bytes());
        // uid at offset 36
        socket_body[36..40].copy_from_slice(&501u32.to_ne_bytes());
        // pid at offset 68
        socket_body[68..72].copy_from_slice(&1234i32.to_ne_bytes());
        // effective_pid at offset 72
        socket_body[72..76].copy_from_slice(&1234i32.to_ne_bytes());
        buf.extend_from_slice(&make_tagged_record(XSO_SOCKET, &socket_body));

        // xinpcb_n body — needs to have vflag and addresses
        let mut inpcb_body = vec![0u8; 120];
        // vflag at offset 44 = INP_IPV4
        inpcb_body[44] = INP_IPV4;
        // local port at offset 72 (network byte order) = 8080
        inpcb_body[72..74].copy_from_slice(&8080u16.to_be_bytes());
        // foreign port at offset 70 (network byte order) = 443
        inpcb_body[70..72].copy_from_slice(&443u16.to_be_bytes());
        // local addr at offset 84 = 127.0.0.1
        inpcb_body[84..88].copy_from_slice(&[127, 0, 0, 1]);
        // foreign addr at offset 80 = 10.0.0.1
        inpcb_body[80..84].copy_from_slice(&[10, 0, 0, 1]);
        buf.extend_from_slice(&make_tagged_record(XSO_INPCB, &inpcb_body));

        // xtcpcb_n body — state = ESTABLISHED (4 on macOS)
        let mut tcpcb_body = vec![0u8; 80];
        // state at offset 0 = 4 (TCPS_ESTABLISHED)
        tcpcb_body[0..4].copy_from_slice(&4i32.to_ne_bytes());
        // t_srtt at offset 24 = 800 (100 ticks << 3 = 800, 100 ticks * 10ms = 1s = 1_000_000us)
        // For hz=100: (800 >> 3) * 1_000_000 / 100 = 100 * 10_000 = 1_000_000
        tcpcb_body[24..28].copy_from_slice(&800u32.to_ne_bytes());
        // snd_cwnd at offset 12
        tcpcb_body[12..16].copy_from_slice(&65535u32.to_ne_bytes());
        buf.extend_from_slice(&make_tagged_record(XSO_TCPCB, &tcpcb_body));

        // End marker: zero-length record
        buf.extend_from_slice(&0u32.to_ne_bytes());
        buf.extend_from_slice(&0u32.to_ne_bytes());

        let result = parse_pcblist_n(&buf, 100).unwrap();
        assert_eq!(result.len(), 1);

        let rec = &result[0];
        assert_eq!(rec.pid, Some(1234));
        assert_eq!(rec.uid, Some(501));
        assert_eq!(rec.ip_version, Some(4));
        assert_eq!(rec.local_port, Some(8080));
        assert_eq!(rec.remote_port, Some(443));
        assert_eq!(rec.local_addr, Some(IpAddr::V4([127, 0, 0, 1])));
        assert_eq!(rec.remote_addr, Some(IpAddr::V4([10, 0, 0, 1])));
        assert_eq!(rec.state, Some(4));
        assert_eq!(rec.snd_cwnd, Some(65535));
        assert_eq!(rec.rtt_us, Some(1_000_000));
        assert_eq!(rec.sources, vec![1]); // DataSource::MacosPcblistN
    }
}
