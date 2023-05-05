use std::ptr::NonNull;
use std::sync::Arc;
use std::{fmt, io, mem, ptr};

use super::cq::Cq;
use super::gid::Gid;
use super::mr::*;
use super::pd::Pd;
use super::wr::*;

use anyhow::Result;
use rdma_sys::*;

/// Queue pair type.
#[derive(fmt::Debug, Clone, Copy, PartialEq, Eq)]
pub enum QpType {
    /// Reliable connection
    RC,
    /// Unreliable datagram
    UD,
}

impl From<QpType> for u32 {
    fn from(qp_type: QpType) -> Self {
        match qp_type {
            QpType::RC => ibv_qp_type::IBV_QPT_RC,
            QpType::UD => ibv_qp_type::IBV_QPT_UD,
        }
    }
}

impl From<u32> for QpType {
    fn from(qp_type: u32) -> Self {
        match qp_type {
            ibv_qp_type::IBV_QPT_RC => QpType::RC,
            ibv_qp_type::IBV_QPT_UD => QpType::UD,
            _ => panic!("invalid qp type"),
        }
    }
}

/// Queue pair state.
#[derive(fmt::Debug, Clone, Copy, PartialEq, Eq)]
pub enum QpState {
    /// Reset
    Reset,
    /// Init
    Init,
    /// Ready to receive
    Rtr,
    /// Ready to send
    Rts,
    /// Error
    Error,
}

impl From<QpState> for u32 {
    fn from(qp_state: QpState) -> Self {
        match qp_state {
            QpState::Reset => ibv_qp_state::IBV_QPS_RESET,
            QpState::Init => ibv_qp_state::IBV_QPS_INIT,
            QpState::Rtr => ibv_qp_state::IBV_QPS_RTR,
            QpState::Rts => ibv_qp_state::IBV_QPS_RTS,
            QpState::Error => ibv_qp_state::IBV_QPS_ERR,
        }
    }
}

impl From<u32> for QpState {
    fn from(qp_state: u32) -> Self {
        match qp_state {
            ibv_qp_state::IBV_QPS_RESET => QpState::Reset,
            ibv_qp_state::IBV_QPS_INIT => QpState::Init,
            ibv_qp_state::IBV_QPS_RTR => QpState::Rtr,
            ibv_qp_state::IBV_QPS_RTS => QpState::Rts,
            ibv_qp_state::IBV_QPS_ERR => QpState::Error,
            _ => panic!("invalid qp state"),
        }
    }
}

/// Queue pair capability attributes.
#[derive(fmt::Debug, Clone, Copy)]
pub struct QpCaps {
    /// The maximum number of outstanding Work Requests that can be posted to
    /// the Send Queue in that Queue Pair.
    ///
    /// Value can be [0..`dev_cap.max_qp_wr`].
    ///
    /// **NOTE:** There may be RDMA devices that for specific transport types
    /// may support less outstanding Work Requests than the maximum reported
    /// value.
    pub max_send_wr: u32,

    /// The maximum number of outstanding Work Requests that can be posted to
    /// the Receive Queue in that Queue Pair.
    ///
    /// Value can be [0..`dev_cap.max_qp_wr`].
    ///
    /// **NOTE:** There may be RDMA devices that for specific transport types
    /// may support less outstanding Work Requests than the maximum reported
    /// value. This value is ignored if the Queue Pair is associated with an SRQ.
    pub max_recv_wr: u32,

    /// The maximum number of scatter/gather elements in any Work Request that
    /// can be posted to the Send Queue in that Queue Pair.
    ///
    /// Value can be [0..`dev_cap.max_sge`].
    ///
    /// **NOTE:** There may be RDMA devices that for specific transport types
    /// may support less scatter/gather elements than the maximum reported value.
    pub max_send_sge: u32,

    /// The maximum number of scatter/gather elements in any Work Request that
    /// can be posted to the Receive Queue in that Queue Pair.
    ///
    /// Value can be [0..`dev_cap.max_sge`].
    ///
    /// **NOTE:** There may be RDMA devices that for specific transport types
    /// may support less scatter/gather elements than the maximum reported value.
    /// This value is ignored if the Queue Pair is associated with an SRQ.
    pub max_recv_sge: u32,

    /// The maximum message size (in bytes) that can be posted inline to the
    /// Send Queue. If no inline message is requested, the value can be 0.
    pub max_inline_data: u32,
}

/// Generate a default RDMA queue pair capabilities setting.
/// The queue pair can:
/// - maintain up to 128 outstanding send/recv work requests each,
/// - set a SGE of up to 16 entries per send/recv work request, and
/// - send up to 64 bytes of inline data.
impl Default for QpCaps {
    fn default() -> Self {
        QpCaps {
            max_send_wr: 128,
            max_recv_wr: 128,
            max_send_sge: 16,
            max_recv_sge: 16,
            max_inline_data: 64,
        }
    }
}

impl QpCaps {
    pub fn new(
        max_send_wr: u32,
        max_recv_wr: u32,
        max_send_sge: u32,
        max_recv_sge: u32,
        max_inline_data: u32,
    ) -> Self {
        QpCaps {
            max_send_wr,
            max_recv_wr,
            max_send_sge,
            max_recv_sge,
            max_inline_data,
        }
    }
}

/// Queue pair initialization attributes.
#[derive(Debug, Clone)]
pub struct QpInitAttr<'a> {
    pub send_cq: Arc<Cq<'a>>,
    pub recv_cq: Arc<Cq<'a>>,
    pub cap: QpCaps,
    pub qp_type: QpType,
    pub sq_sig_all: bool,
}

impl<'a> QpInitAttr<'a> {
    pub fn new(
        send_cq: Arc<Cq<'a>>,
        recv_cq: Arc<Cq<'a>>,
        cap: QpCaps,
        qp_type: QpType,
        sq_sig_all: bool,
    ) -> Self {
        QpInitAttr {
            send_cq,
            recv_cq,
            cap,
            qp_type,
            sq_sig_all,
        }
    }
}

impl<'a> From<QpInitAttr<'a>> for ibv_qp_init_attr {
    fn from(value: QpInitAttr<'a>) -> Self {
        ibv_qp_init_attr {
            qp_context: ptr::null_mut(),
            send_cq: value.send_cq.as_ptr(),
            recv_cq: value.recv_cq.as_ptr(),
            srq: ptr::null_mut(),
            cap: ibv_qp_cap {
                max_send_wr: value.cap.max_send_wr,
                max_recv_wr: value.cap.max_recv_wr,
                max_send_sge: value.cap.max_send_sge,
                max_recv_sge: value.cap.max_recv_sge,
                max_inline_data: value.cap.max_inline_data,
            },
            qp_type: u32::from(value.qp_type),
            sq_sig_all: value.sq_sig_all as i32,
        }
    }
}

/// Endpoint (NIC port & queue pair) data.
#[derive(fmt::Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct QpEndpoint {
    pub gid: Gid,
    pub port_num: u8,
    pub lid: u16,
    pub qpn: u32,
    pub psn: u32,
    pub qkey: u32,
}

impl QpEndpoint {
    pub fn new(gid: Gid, port_num: u8, lid: u16, qpn: u32, psn: u32, qkey: u32) -> Self {
        QpEndpoint {
            gid,
            port_num,
            lid,
            qpn,
            psn,
            qkey,
        }
    }
}

/// Peer queue pair information that can be used in sends.
pub struct QpPeer {
    pub ah: NonNull<ibv_ah>,
    pub ep: QpEndpoint,
}

unsafe impl Sync for QpPeer {}

impl QpPeer {
    pub fn new(pd: &Pd, ep: QpEndpoint) -> Result<Self> {
        let mut ah_attr = ibv_ah_attr {
            grh: ibv_global_route {
                dgid: ibv_gid::from(ep.gid),
                flow_label: 0,
                sgid_index: pd.context().gid_index(),
                hop_limit: 0xFF,
                traffic_class: 0,
            },
            is_global: 1,
            dlid: ep.lid,
            sl: 0,
            src_path_bits: 0,
            static_rate: 0,
            port_num: ep.port_num,
        };
        let ah = NonNull::new(unsafe { ibv_create_ah(pd.as_ptr(), &mut ah_attr) })
            .ok_or_else(|| anyhow::anyhow!(io::Error::last_os_error()))?;
        Ok(QpPeer { ah, ep })
    }
}

impl From<&QpPeer> for ud_t {
    fn from(peer: &QpPeer) -> Self {
        ud_t {
            ah: peer.ah.as_ptr(),
            remote_qpn: peer.ep.qpn,
            remote_qkey: peer.ep.qkey,
        }
    }
}

impl Drop for QpPeer {
    fn drop(&mut self) {
        unsafe { ibv_destroy_ah(self.ah.as_ptr()) };
    }
}

/// Queue pair.
pub struct Qp<'a> {
    pd: &'a Pd<'a>,
    qp: NonNull<ibv_qp>,
    init_attr: QpInitAttr<'a>,
}

unsafe impl<'a> Sync for Qp<'a> {}

impl<'a> From<&Qp<'a>> for QpEndpoint {
    fn from(qp: &Qp<'a>) -> Self {
        const GLOBAL_QKEY: u32 = 0x11111111;
        QpEndpoint {
            gid: qp.pd().context().gid(),
            port_num: qp.pd().context().port_num(),
            lid: qp.pd().context().lid(),
            qpn: qp.qp_num(),
            psn: 0,
            qkey: GLOBAL_QKEY,
        }
    }
}

impl<'a> Qp<'a> {
    pub fn new(pd: &'a Pd, attr: QpInitAttr<'a>) -> Result<Self> {
        let qp = NonNull::new(unsafe {
            ibv_create_qp(pd.as_ptr(), &mut ibv_qp_init_attr::from(attr.clone()))
        })
        .ok_or_else(|| anyhow::anyhow!(io::Error::last_os_error()))?;
        Ok(Qp {
            pd,
            qp,
            init_attr: attr,
        })
    }

    #[inline]
    pub fn as_ptr(&self) -> *mut ibv_qp {
        self.qp.as_ptr()
    }

    #[inline]
    pub fn pd(&self) -> &Pd {
        self.pd
    }

    #[inline]
    pub fn qp_type(&self) -> QpType {
        let ty = unsafe { (*self.qp.as_ptr()).qp_type };
        match ty {
            ibv_qp_type::IBV_QPT_RC => QpType::RC,
            ibv_qp_type::IBV_QPT_UD => QpType::UD,
            _ => panic!("unknown qp type"),
        }
    }

    #[inline]
    pub(crate) fn qp_num(&self) -> u32 {
        unsafe { (*self.qp.as_ptr()).qp_num }
    }

    #[inline]
    pub fn state(&self) -> QpState {
        let state = unsafe { (*self.qp.as_ptr()).state };
        match state {
            ibv_qp_state::IBV_QPS_RESET => QpState::Reset,
            ibv_qp_state::IBV_QPS_INIT => QpState::Init,
            ibv_qp_state::IBV_QPS_RTR => QpState::Rtr,
            ibv_qp_state::IBV_QPS_RTS => QpState::Rts,
            ibv_qp_state::IBV_QPS_ERR => QpState::Error,
            _ => panic!("unknown qp state"),
        }
    }

    #[inline]
    pub fn scq(&self) -> Arc<Cq<'a>> {
        self.init_attr.send_cq.clone()
    }

    #[inline]
    pub fn rcq(&self) -> Arc<Cq<'a>> {
        self.init_attr.recv_cq.clone()
    }

    #[inline]
    pub fn scq_as_ref(&self) -> &Cq<'a> {
        &self.init_attr.send_cq
    }

    #[inline]
    pub fn rcq_as_ref(&self) -> &Cq<'a> {
        &self.init_attr.recv_cq
    }

    fn modify_reset_to_init(&self, ep: &QpEndpoint) -> Result<()> {
        let mut attr = unsafe { mem::zeroed::<ibv_qp_attr>() };
        let mut attr_mask = ibv_qp_attr_mask::IBV_QP_STATE
            | ibv_qp_attr_mask::IBV_QP_PKEY_INDEX
            | ibv_qp_attr_mask::IBV_QP_PORT;
        attr.qp_state = ibv_qp_state::IBV_QPS_INIT;
        attr.pkey_index = 0;
        attr.port_num = ep.port_num;

        if self.qp_type() == QpType::RC {
            attr.qp_access_flags = (ibv_access_flags::IBV_ACCESS_REMOTE_WRITE
                | ibv_access_flags::IBV_ACCESS_REMOTE_READ
                | ibv_access_flags::IBV_ACCESS_REMOTE_ATOMIC)
                .0;
            attr_mask = attr_mask | ibv_qp_attr_mask::IBV_QP_ACCESS_FLAGS;
        } else {
            attr_mask = attr_mask | ibv_qp_attr_mask::IBV_QP_QKEY;
            attr.qkey = ep.qkey;
        }

        let ret = unsafe { ibv_modify_qp(self.qp.as_ptr(), &mut attr, attr_mask.0 as i32) };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    fn modify_init_to_rtr(&self, ep: &QpEndpoint) -> Result<()> {
        let mut attr = unsafe { mem::zeroed::<ibv_qp_attr>() };
        let mut attr_mask = ibv_qp_attr_mask::IBV_QP_STATE;
        attr.qp_state = ibv_qp_state::IBV_QPS_RTR;

        if self.qp_type() == QpType::RC {
            let ctx = self.pd.context();

            attr.path_mtu = ctx.active_mtu();
            attr.dest_qp_num = ep.qpn;
            attr.rq_psn = ep.psn;
            attr.max_dest_rd_atomic = 16;
            attr.min_rnr_timer = 12;

            attr.ah_attr.grh.dgid = ibv_gid::from(ep.gid);
            attr.ah_attr.grh.flow_label = 0;
            attr.ah_attr.grh.sgid_index = ctx.gid_index();
            attr.ah_attr.grh.hop_limit = 0xFF;
            attr.ah_attr.grh.traffic_class = 0;
            attr.ah_attr.dlid = ep.lid;
            attr.ah_attr.sl = 0;
            attr.ah_attr.src_path_bits = 0;
            attr.ah_attr.port_num = ctx.port_num();
            attr.ah_attr.is_global = 1;

            attr_mask = attr_mask
                | ibv_qp_attr_mask::IBV_QP_AV
                | ibv_qp_attr_mask::IBV_QP_PATH_MTU
                | ibv_qp_attr_mask::IBV_QP_DEST_QPN
                | ibv_qp_attr_mask::IBV_QP_RQ_PSN
                | ibv_qp_attr_mask::IBV_QP_MAX_DEST_RD_ATOMIC
                | ibv_qp_attr_mask::IBV_QP_MIN_RNR_TIMER;
        }

        let ret = unsafe { ibv_modify_qp(self.qp.as_ptr(), &mut attr, attr_mask.0 as i32) };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    fn modify_rtr_to_rts(&self, ep: &QpEndpoint) -> Result<()> {
        let mut attr = unsafe { mem::zeroed::<ibv_qp_attr>() };
        let mut attr_mask = ibv_qp_attr_mask::IBV_QP_STATE | ibv_qp_attr_mask::IBV_QP_SQ_PSN;
        attr.qp_state = ibv_qp_state::IBV_QPS_RTS;
        attr.sq_psn = ep.psn;

        if self.qp_type() == QpType::RC {
            attr.max_rd_atomic = 16;
            attr.timeout = 14;
            attr.retry_cnt = 6;
            attr.rnr_retry = 6;
            attr_mask = attr_mask
                | ibv_qp_attr_mask::IBV_QP_MAX_QP_RD_ATOMIC
                | ibv_qp_attr_mask::IBV_QP_TIMEOUT
                | ibv_qp_attr_mask::IBV_QP_RETRY_CNT
                | ibv_qp_attr_mask::IBV_QP_RNR_RETRY;
        }

        let ret = unsafe { ibv_modify_qp(self.qp.as_ptr(), &mut attr, attr_mask.0 as i32) };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    pub fn connect(&self, ep: &QpEndpoint) -> Result<()> {
        if self.state() == QpState::Reset {
            self.modify_reset_to_init(ep)?;
        }
        if self.state() == QpState::Init {
            self.modify_init_to_rtr(ep)?;
        }
        if self.state() == QpState::Rtr {
            self.modify_rtr_to_rts(ep)?;
        }
        Ok(())
    }

    /// Post a RDMA recv using the given buffer array.
    ///
    /// **NOTE:** This method has no mutable borrows it its parameters, but can
    /// cause the content of the buffers to be modified!
    pub fn recv(&self, local: &[MrSlice<'_>], wr_id: u64) -> Result<()> {
        let mut sgl = build_sgl(local);
        let mut wr = ibv_recv_wr {
            wr_id,
            next: ptr::null_mut(),
            sg_list: if local.len() == 0 {
                ptr::null_mut()
            } else {
                sgl.as_mut_ptr()
            },
            num_sge: local.len() as i32,
        };
        let ret = unsafe {
            let mut bad_wr = ptr::null_mut();
            ibv_post_recv(self.qp.as_ptr(), &mut wr, &mut bad_wr)
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    pub fn send(&self, local: &[MrSlice<'_>], wr_id: u64, signal: bool) -> Result<()> {
        let mut sgl = build_sgl(local);
        let mut wr = unsafe { mem::zeroed::<ibv_send_wr>() };
        wr = ibv_send_wr {
            wr_id,
            next: ptr::null_mut(),
            sg_list: if local.len() == 0 {
                ptr::null_mut()
            } else {
                sgl.as_mut_ptr()
            },
            num_sge: local.len() as i32,
            opcode: ibv_wr_opcode::IBV_WR_SEND,
            send_flags: if signal {
                ibv_send_flags::IBV_SEND_SIGNALED.0
            } else {
                0
            },
            ..wr
        };
        let ret = unsafe {
            let mut bad_wr = ptr::null_mut();
            ibv_post_send(self.qp.as_ptr(), &mut wr, &mut bad_wr)
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    pub fn send_to(
        &self,
        peer: &QpPeer,
        local: &[MrSlice<'_>],
        wr_id: u64,
        signal: bool,
    ) -> Result<()> {
        let mut sgl = build_sgl(local);
        let mut wr = unsafe { mem::zeroed::<ibv_send_wr>() };
        wr = ibv_send_wr {
            wr_id,
            next: ptr::null_mut(),
            sg_list: if local.len() == 0 {
                ptr::null_mut()
            } else {
                sgl.as_mut_ptr()
            },
            num_sge: local.len() as i32,
            opcode: ibv_wr_opcode::IBV_WR_SEND,
            send_flags: if signal {
                ibv_send_flags::IBV_SEND_SIGNALED.0
            } else {
                0
            },
            ..wr
        };
        wr.wr.ud = ud_t::from(peer);
        let ret = unsafe {
            let mut bad_wr = ptr::null_mut();
            ibv_post_send(self.qp.as_ptr(), &mut wr, &mut bad_wr)
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    pub fn read(
        &self,
        local: &[MrSlice<'_>],
        remote: &RemoteMrSlice<'_>,
        wr_id: u64,
        signal: bool,
    ) -> Result<()> {
        let mut sgl = build_sgl(local);
        let mut wr = unsafe { mem::zeroed::<ibv_send_wr>() };
        wr = ibv_send_wr {
            wr_id,
            next: ptr::null_mut(),
            sg_list: if local.len() == 0 {
                ptr::null_mut()
            } else {
                sgl.as_mut_ptr()
            },
            num_sge: local.len() as i32,
            opcode: ibv_wr_opcode::IBV_WR_RDMA_READ,
            send_flags: if signal {
                ibv_send_flags::IBV_SEND_SIGNALED.0
            } else {
                0
            },
            wr: wr_t {
                rdma: rdma_t::from(remote),
            },
            ..wr
        };
        let ret = unsafe {
            let mut bad_wr = ptr::null_mut();
            ibv_post_send(self.qp.as_ptr(), &mut wr, &mut bad_wr)
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    pub fn write(
        &self,
        local: &[MrSlice<'_>],
        remote: &RemoteMrSlice<'_>,
        wr_id: u64,
        imm: Option<u32>,
        signal: bool,
    ) -> Result<()> {
        let mut sgl = build_sgl(local);
        let mut wr = unsafe { mem::zeroed::<ibv_send_wr>() };
        wr = ibv_send_wr {
            wr_id,
            next: ptr::null_mut(),
            sg_list: if local.len() == 0 {
                ptr::null_mut()
            } else {
                sgl.as_mut_ptr()
            },
            num_sge: local.len() as i32,
            opcode: if imm.is_none() {
                ibv_wr_opcode::IBV_WR_RDMA_WRITE
            } else {
                ibv_wr_opcode::IBV_WR_RDMA_WRITE_WITH_IMM
            },
            send_flags: if signal {
                ibv_send_flags::IBV_SEND_SIGNALED.0
            } else {
                0
            },
            imm_data_invalidated_rkey_union: imm_data_invalidated_rkey_union_t {
                imm_data: imm.unwrap_or(0),
            },
            wr: wr_t {
                rdma: rdma_t::from(remote),
            },
            ..wr
        };
        let ret = unsafe {
            let mut bad_wr = ptr::null_mut();
            ibv_post_send(self.qp.as_ptr(), &mut wr, &mut bad_wr)
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    #[inline]
    pub fn post_send(&self, ops: &[SendWr<'_>]) -> Result<()> {
        // Safety: we only hold references to the `SendWr`s, whose lifetimes
        // can only outlive this function. `ibv_post_send` is used inside this
        // function, so the work requests are guaranteed to be valid.
        let mut wrs = ops
            .iter()
            .map(|op| unsafe { op.to_wr() })
            .collect::<Vec<_>>();
        for i in 0..(wrs.len() - 1) {
            wrs[i].next = &mut wrs[i + 1];
        }

        let ret = unsafe {
            let mut bad_wr = ptr::null_mut();
            ibv_post_send(self.qp.as_ptr(), wrs.as_mut_ptr(), &mut bad_wr)
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }

    #[inline]
    pub fn post_recv(&self, ops: &[RecvWr]) -> Result<()> {
        // Safety: we only hold references to the `RecvWr`s, whose lifetimes
        // can only outlive this function. `ibv_post_recv` is used inside this
        // function, so the work requests are guaranteed to be valid.
        let mut wrs = ops
            .iter()
            .map(|op| unsafe { op.to_wr() })
            .collect::<Vec<_>>();
        for i in 0..(wrs.len() - 1) {
            wrs[i].next = &mut wrs[i + 1];
        }

        let ret = unsafe {
            let mut bad_wr = ptr::null_mut();
            ibv_post_recv(self.qp.as_ptr(), wrs.as_mut_ptr(), &mut bad_wr)
        };
        if ret == 0 {
            Ok(())
        } else {
            Err(anyhow::anyhow!(io::Error::last_os_error()))
        }
    }
}

impl<'a> Drop for Qp<'a> {
    fn drop(&mut self) {
        unsafe {
            ibv_destroy_qp(self.qp.as_ptr());
        }
    }
}

#[inline]
pub(crate) fn build_sgl<'a>(slices: &'a [MrSlice<'a>]) -> Vec<ibv_sge> {
    slices
        .iter()
        .map(|slice| ibv_sge {
            addr: slice.mr().addr() as u64 + slice.offset() as u64,
            length: slice.len() as u32,
            lkey: slice.mr().lkey(),
        })
        .collect()
}
