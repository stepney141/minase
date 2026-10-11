use super::{Device, Error, Result, device::pinned_zeros, require_len};
use crate::data::{
    BatchSlot, CANONICAL_ROWS, FEATURE_COLUMNS, InputFeatureSet, MAX_BATCH_SIZE, max_segment_count,
};
use crate::model::Shape;
use cudarc::driver::{
    CudaEvent, CudaSlice, DeviceRepr, PinnedHostSlice, ValidAsZeroBits, sys::CUevent_flags,
};
use std::sync::Arc;

/// 投入されたデバイス面を識別する券。同じ環の、完了を記録する前の券だけが有効。
#[derive(Clone, Debug)]
pub struct BatchTicket {
    owner: Arc<()>,
    generation: u64,
    host: usize,
    device: usize,
}

struct TransferEvents {
    start: CudaEvent,
    end: CudaEvent,
}

/// 1回の転送の時刻。面の再利用後も同じ転送を測定できる。
pub struct TransferTiming {
    events: Arc<TransferEvents>,
}
impl TransferTiming {
    /// 転送の完了を待ち、転送ストリーム上で測ったミリ秒を返す。
    /// 次の更新を投入してから呼ぶと、計算とホスト処理を重ねられる。
    pub fn collect(self) -> Result<f32> {
        Ok(self.events.start.elapsed_ms(&self.events.end)?)
    }
}

// CUDAを必要としない状態遷移。完了イベントを記録するまでは面を再予約しない。
struct RingState {
    owner: Arc<()>,
    next: u64,
    active: [Option<u64>; 2],
    poisoned: bool,
}
impl RingState {
    fn new() -> Self {
        Self {
            owner: Arc::new(()),
            next: 0,
            active: [None; 2],
            poisoned: false,
        }
    }
    fn reserve(&mut self) -> Result<BatchTicket> {
        if self.poisoned {
            return Err(Error::Invalid("ring has a failed transfer"));
        }
        let generation = self.next;
        let device = (generation % 2) as usize;
        if self.active[device].is_some() {
            return Err(Error::Invalid("backward completion not recorded"));
        }
        self.next = self
            .next
            .checked_add(1)
            .ok_or(Error::Invalid("ring generation overflow"))?;
        self.active[device] = Some(generation);
        Ok(BatchTicket {
            owner: self.owner.clone(),
            generation,
            host: (generation % 3) as usize,
            device,
        })
    }
    fn check(&self, ticket: &BatchTicket) -> Result<()> {
        if self.poisoned
            || !Arc::ptr_eq(&self.owner, &ticket.owner)
            || self.active[ticket.device] != Some(ticket.generation)
        {
            return Err(Error::Invalid("stale, foreign, or failed batch ticket"));
        }
        Ok(())
    }
    fn finish(&mut self, ticket: &BatchTicket) -> Result<()> {
        self.check(ticket)?;
        self.active[ticket.device] = None;
        Ok(())
    }
}

/// ワーカーが所有して埋められるページ固定メモリの面。
///
/// 所有権を供給スレッドと計算スレッドの間で移し、`fill`で前回転送の完了を待つ。
/// 元の`HostRing`だけがこの面を転送でき、転送中のメモリを同時に書き換えられない。
pub struct HostBuffer {
    owner: Arc<()>,
    set: InputFeatureSet,
    capacity: usize,
    record_indices: PinnedHostSlice<u64>,
    features: PinnedHostSlice<u32>,
    linear_features: PinnedHostSlice<u32>,
    piece_counts: PinnedHostSlice<u8>,
    teacher_scores: PinnedHostSlice<f32>,
    teacher_classes: PinnedHostSlice<u32>,
    outcomes: PinnedHostSlice<u8>,
    first_segment_starts: PinnedHostSlice<u32>,
    first_segment_offsets: PinnedHostSlice<u32>,
    first_entries: PinnedHostSlice<u32>,
    linear_segment_starts: PinnedHostSlice<u32>,
    linear_segment_offsets: PinnedHostSlice<u32>,
    linear_entries: PinnedHostSlice<u32>,
    len: usize,
    first_len: usize,
    linear_len: usize,
    first_segments: usize,
    linear_segments: usize,
    transfer_done: CudaEvent,
}
impl HostBuffer {
    fn new(device: &Device, set: InputFeatureSet, capacity: usize, owner: Arc<()>) -> Result<Self> {
        let entries = capacity * 2 * FEATURE_COLUMNS;
        Ok(Self {
            owner,
            set,
            capacity,
            record_indices: pinned_zeros(device, capacity)?,
            features: pinned_zeros(device, entries)?,
            linear_features: pinned_zeros(device, capacity * FEATURE_COLUMNS)?,
            piece_counts: pinned_zeros(device, capacity)?,
            teacher_scores: pinned_zeros(device, capacity)?,
            teacher_classes: pinned_zeros(device, capacity)?,
            outcomes: pinned_zeros(device, capacity)?,
            first_segment_starts: pinned_zeros(device, set.first_layer_rows() + 1)?,
            first_segment_offsets: pinned_zeros(
                device,
                max_segment_count(set.first_layer_rows(), entries) + 1,
            )?,
            first_entries: pinned_zeros(device, entries)?,
            linear_segment_starts: pinned_zeros(device, CANONICAL_ROWS + 1)?,
            linear_segment_offsets: pinned_zeros(
                device,
                max_segment_count(CANONICAL_ROWS, capacity * FEATURE_COLUMNS) + 1,
            )?,
            linear_entries: pinned_zeros(device, entries)?,
            len: 0,
            first_len: 0,
            linear_len: 0,
            first_segments: 0,
            linear_segments: 0,
            transfer_done: device.context.new_event(None)?,
        })
    }
    /// 前回転送の完了を待ち、バッチの全配列をこの面へ写す。
    pub fn fill(&mut self, batch: &BatchSlot) -> Result<()> {
        if batch.feature_set() != self.set || batch.is_empty() || batch.len() > self.capacity {
            return Err(Error::Invalid("batch feature set or host buffer capacity"));
        }
        self.len = 0;
        self.transfer_done.synchronize()?;
        copy(&mut self.record_indices, batch.record_indices())?;
        copy(&mut self.features, batch.features())?;
        copy(&mut self.linear_features, batch.linear_features())?;
        copy(&mut self.piece_counts, batch.piece_counts())?;
        copy(&mut self.teacher_scores, batch.teacher_scores())?;
        copy(&mut self.teacher_classes, batch.teacher_classes())?;
        copy(&mut self.outcomes, batch.outcomes())?;
        copy(
            &mut self.first_segment_starts,
            batch.first_layer_inverse().segment_starts(),
        )?;
        copy(
            &mut self.first_segment_offsets,
            batch.first_layer_inverse().segment_offsets(),
        )?;
        copy(
            &mut self.first_entries,
            batch.first_layer_inverse().entries(),
        )?;
        copy(
            &mut self.linear_segment_starts,
            batch.linear_inverse().segment_starts(),
        )?;
        copy(
            &mut self.linear_segment_offsets,
            batch.linear_inverse().segment_offsets(),
        )?;
        copy(&mut self.linear_entries, batch.linear_inverse().entries())?;
        self.len = batch.len();
        self.first_len = batch.first_layer_inverse().entries().len();
        self.linear_len = batch.linear_inverse().entries().len();
        self.first_segments = batch.first_layer_inverse().segment_count();
        self.linear_segments = batch.linear_inverse().segment_count();
        Ok(())
    }

    fn transfer_bytes(&self) -> usize {
        self.record_indices.len() * size_of::<u64>()
            + self.piece_counts.len() * size_of::<u8>()
            + self.teacher_scores.len() * size_of::<f32>()
            + self.outcomes.len() * size_of::<u8>()
            + (self.features.len()
                + self.linear_features.len()
                + self.teacher_classes.len()
                + self.first_segment_starts.len()
                + self.first_segment_offsets.len()
                + self.first_entries.len()
                + self.linear_segment_starts.len()
                + self.linear_segment_offsets.len()
                + self.linear_entries.len())
                * size_of::<u32>()
    }
}
fn copy<T: Copy + ValidAsZeroBits>(dst: &mut PinnedHostSlice<T>, src: &[T]) -> Result<()> {
    require_len(dst.len(), src.len())?;
    dst.as_mut_slice()?[..src.len()].copy_from_slice(src);
    Ok(())
}

pub(super) struct DeviceInput {
    pub record_indices: CudaSlice<u64>,
    pub features: CudaSlice<u32>,
    pub linear_features: CudaSlice<u32>,
    pub piece_counts: CudaSlice<u8>,
    pub teacher_scores: CudaSlice<f32>,
    pub teacher_classes: CudaSlice<u32>,
    pub outcomes: CudaSlice<u8>,
    pub first_segment_starts: CudaSlice<u32>,
    pub first_segment_offsets: CudaSlice<u32>,
    pub first_entries: CudaSlice<u32>,
    pub linear_segment_starts: CudaSlice<u32>,
    pub linear_segment_offsets: CudaSlice<u32>,
    pub linear_entries: CudaSlice<u32>,
    pub len: usize,
    pub first_len: usize,
    pub linear_len: usize,
    pub first_segments: usize,
    pub linear_segments: usize,
    backward_done: CudaEvent,
}
impl DeviceInput {
    fn new(device: &Device, set: InputFeatureSet, capacity: usize) -> Result<Self> {
        let s = &device.transfer;
        let entries = capacity * 2 * FEATURE_COLUMNS;
        Ok(Self {
            record_indices: s.alloc_zeros(capacity)?,
            features: s.alloc_zeros(entries)?,
            linear_features: s.alloc_zeros(capacity * FEATURE_COLUMNS)?,
            piece_counts: s.alloc_zeros(capacity)?,
            teacher_scores: s.alloc_zeros(capacity)?,
            teacher_classes: s.alloc_zeros(capacity)?,
            outcomes: s.alloc_zeros(capacity)?,
            first_segment_starts: s.alloc_zeros(set.first_layer_rows() + 1)?,
            first_segment_offsets: s
                .alloc_zeros(max_segment_count(set.first_layer_rows(), entries) + 1)?,
            first_entries: s.alloc_zeros(entries)?,
            linear_segment_starts: s.alloc_zeros(CANONICAL_ROWS + 1)?,
            linear_segment_offsets: s
                .alloc_zeros(max_segment_count(CANONICAL_ROWS, capacity * FEATURE_COLUMNS) + 1)?,
            linear_entries: s.alloc_zeros(entries)?,
            len: 0,
            first_len: 0,
            linear_len: 0,
            first_segments: 0,
            linear_segments: 0,
            backward_done: device.context.new_event(None)?,
        })
    }
}

/// 3つのページ固定ホスト面と2つのデバイス入力面からなる転送の環。
///
/// `upload`は投入順に面を選ぶ。`finish`で計算ストリームの最後の使用を記録してから、
/// 同じデバイス面を再利用する。第1層と線形経路の後退も、その面の使用に含める。
pub struct HostRing {
    device: Device,
    set: InputFeatureSet,
    capacity: usize,
    hosts: Option<[HostBuffer; 3]>,
    transfer_bytes: usize,
    inputs: [DeviceInput; 2],
    timings: [Option<Arc<TransferEvents>>; 2],
    state: RingState,
}
impl HostRing {
    /// 最大局面数に合わせて固定メモリと入力配列を確保する。
    pub fn new(device: &Device, set: InputFeatureSet, capacity: usize) -> Result<Self> {
        if !(1..=MAX_BATCH_SIZE).contains(&capacity) {
            return Err(Error::Invalid("ring capacity"));
        }
        let state = RingState::new();
        let hosts = [
            HostBuffer::new(device, set, capacity, state.owner.clone())?,
            HostBuffer::new(device, set, capacity, state.owner.clone())?,
            HostBuffer::new(device, set, capacity, state.owner.clone())?,
        ];
        let transfer_bytes = hosts[0].transfer_bytes();
        Ok(Self {
            device: device.clone(),
            set,
            capacity,
            hosts: Some(hosts),
            transfer_bytes,
            inputs: [
                DeviceInput::new(device, set, capacity)?,
                DeviceInput::new(device, set, capacity)?,
            ],
            timings: [None, None],
            state,
        })
    }
    /// BatchSlotを固定メモリへ写し、転送ストリームへ転送を投入する。
    /// 計算ストリームには転送完了イベントの待機を投入する。
    pub fn upload(&mut self, batch: &BatchSlot) -> Result<BatchTicket> {
        if batch.feature_set() != self.set || batch.is_empty() || batch.len() > self.capacity {
            return Err(Error::Invalid("batch feature set or ring capacity"));
        }
        let hosts = self
            .hosts
            .as_mut()
            .ok_or(Error::Invalid("host buffers are owned by producers"))?;
        let ticket = self.state.reserve()?;
        let host = &mut hosts[ticket.host];
        let transfer = host
            .fill(batch)
            .and_then(|()| transfer_buffer(&self.device, host, &mut self.inputs[ticket.device]));
        match transfer {
            Ok(timing) => self.timings[ticket.device] = Some(timing),
            Err(error) => {
                // 部分転送後は再使用を拒否する。Dropは記録済みのCUDA使用を待つ。
                self.state.poisoned = true;
                return Err(error);
            }
        }
        Ok(ticket)
    }

    /// 最初の投入前に3面の所有権をワーカーへ移す。取得は1回だけ許す。
    /// 以降は`upload_prepared`で、埋め終わった面の転送を投入する。
    pub fn take_host_buffers(&mut self) -> Result<[HostBuffer; 3]> {
        if self.state.next != 0 || self.state.poisoned {
            return Err(Error::Invalid(
                "host buffers must be taken before the first upload",
            ));
        }
        self.hosts
            .take()
            .ok_or(Error::Invalid("host buffers already taken"))
    }

    /// ワーカーが埋めた面を転送する。ホスト側のコピーは行わない。
    /// 呼出後は面をワーカーへ返せる。次の`fill`が今回の転送完了を待つ。
    pub fn upload_prepared(&mut self, host: &mut HostBuffer) -> Result<BatchTicket> {
        if !Arc::ptr_eq(&self.state.owner, &host.owner)
            || host.set != self.set
            || host.capacity != self.capacity
            || host.len == 0
        {
            return Err(Error::Invalid("foreign or unfilled host buffer"));
        }
        let ticket = self.state.reserve()?;
        let transfer = transfer_buffer(&self.device, host, &mut self.inputs[ticket.device]);
        if transfer.is_err() {
            self.state.poisoned = true;
        }
        self.timings[ticket.device] = Some(transfer?);
        Ok(ticket)
    }

    /// `finish`より前に、券に対応する転送の測定値を予約する。
    pub fn transfer_timing(&self, ticket: &BatchTicket) -> Result<TransferTiming> {
        self.state.check(ticket)?;
        let events = self.timings[ticket.device]
            .as_ref()
            .ok_or(Error::Invalid("batch has no recorded transfer"))?;
        Ok(TransferTiming {
            events: events.clone(),
        })
    }

    /// 1回の`upload`で転送するバイト数。端数バッチでも面の容量全体を送る。
    pub fn transfer_bytes(&self) -> usize {
        self.transfer_bytes
    }

    pub(super) fn validate_device(&self, device: &Device, shape: Shape) -> Result<()> {
        if !Arc::ptr_eq(&self.device.compute, &device.compute)
            || !Arc::ptr_eq(&self.device.transfer, &device.transfer)
            || self.set != shape.feature_set()
        {
            return Err(Error::Invalid("ring device streams or feature set"));
        }
        Ok(())
    }
    /// この面を最後に読む後退または検証を投入した後、完了イベントを記録する。
    pub fn finish(&mut self, ticket: BatchTicket) -> Result<()> {
        self.state.check(&ticket)?;
        self.inputs[ticket.device]
            .backward_done
            .record(&self.device.compute)?;
        self.state.finish(&ticket)
    }
    pub(super) fn input(&self, ticket: &BatchTicket) -> Result<&DeviceInput> {
        self.state.check(ticket)?;
        Ok(&self.inputs[ticket.device])
    }
}

fn transfer_buffer(
    device: &Device,
    host: &mut HostBuffer,
    input: &mut DeviceInput,
) -> Result<Arc<TransferEvents>> {
    let timing = Arc::new(TransferEvents {
        start: device
            .context
            .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
        end: device
            .context
            .new_event(Some(CUevent_flags::CU_EVENT_DEFAULT))?,
    });
    device.transfer.wait(&input.backward_done)?;
    timing.start.record(&device.transfer)?;
    let send = |src: &PinnedHostSlice<u32>, dst: &mut CudaSlice<u32>| transfer(device, src, dst);
    transfer(device, &host.record_indices, &mut input.record_indices)?;
    send(&host.features, &mut input.features)?;
    send(&host.linear_features, &mut input.linear_features)?;
    transfer(device, &host.piece_counts, &mut input.piece_counts)?;
    transfer(device, &host.teacher_scores, &mut input.teacher_scores)?;
    send(&host.teacher_classes, &mut input.teacher_classes)?;
    transfer(device, &host.outcomes, &mut input.outcomes)?;
    send(&host.first_segment_starts, &mut input.first_segment_starts)?;
    send(
        &host.first_segment_offsets,
        &mut input.first_segment_offsets,
    )?;
    send(&host.first_entries, &mut input.first_entries)?;
    send(
        &host.linear_segment_starts,
        &mut input.linear_segment_starts,
    )?;
    send(
        &host.linear_segment_offsets,
        &mut input.linear_segment_offsets,
    )?;
    send(&host.linear_entries, &mut input.linear_entries)?;
    input.len = host.len;
    input.first_len = host.first_len;
    input.linear_len = host.linear_len;
    input.first_segments = host.first_segments;
    input.linear_segments = host.linear_segments;
    timing.end.record(&device.transfer)?;
    host.transfer_done.record(&device.transfer)?;
    device.compute.wait(&host.transfer_done)?;
    Ok(timing)
}

fn transfer<T: DeviceRepr>(
    device: &Device,
    src: &PinnedHostSlice<T>,
    dst: &mut CudaSlice<T>,
) -> Result<()> {
    require_len(dst.len(), src.len())?;
    // PinnedHostSliceのまま渡す。as_sliceを介した同期と通常メモリ転送を避ける。
    device.transfer.memcpy_htod(src, dst)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a CUDA GPU"]
    fn gpu_workers_fill_and_reuse_three_owned_host_buffers() {
        let device = Device::new(0).unwrap();
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let mut ring = HostRing::new(&device, set, 3).unwrap();
            let bytes = ring.transfer_bytes();
            let buffers = ring.take_host_buffers().unwrap();
            assert!(ring.take_host_buffers().is_err());
            assert_eq!(ring.transfer_bytes(), bytes);
            let mut foreign = HostRing::new(&device, set, 3).unwrap();
            let (recycle, producer) = std::sync::mpsc::sync_channel(3);
            let (prepared, consumer) = std::sync::mpsc::sync_channel(3);
            for buffer in buffers {
                recycle.send(buffer).unwrap();
            }
            std::thread::scope(|scope| {
                scope.spawn(move || {
                    for iteration in 0..8 {
                        let batch = super::super::equivalence::synthetic_batch(
                            set,
                            1 + iteration % 3,
                            71 + iteration as u64,
                        )
                        .unwrap();
                        let mut buffer = producer.recv().unwrap();
                        buffer.fill(&batch).unwrap();
                        prepared.send((buffer, batch)).unwrap();
                    }
                });
                for iteration in 0..8 {
                    let (mut buffer, batch) = consumer.recv().unwrap();
                    assert!(foreign.upload_prepared(&mut buffer).is_err());
                    assert!(ring.upload(&batch).is_err());
                    let ticket = ring.upload_prepared(&mut buffer).unwrap();
                    let input = ring.input(&ticket).unwrap();
                    assert_eq!(input.len, batch.len());
                    assert_eq!(
                        device
                            .compute
                            .clone_dtoh(&input.features.slice(..batch.features().len()))
                            .unwrap(),
                        batch.features(),
                    );
                    assert_eq!(
                        device
                            .compute
                            .clone_dtoh(
                                &input
                                    .first_entries
                                    .slice(..batch.first_layer_inverse().entries().len())
                            )
                            .unwrap(),
                        batch.first_layer_inverse().entries(),
                    );
                    ring.finish(ticket).unwrap();
                    // 転送した面自身を返し、4面目を確保せずに再使用する。
                    if iteration < 5 {
                        recycle.send(buffer).unwrap();
                    }
                }
            });
        }
    }

    #[test]
    #[ignore = "requires a CUDA GPU"]
    fn gpu_transfer_timing_survives_reused_input_and_host_slots() {
        let device = Device::new(0).unwrap();
        let batch = super::super::equivalence::synthetic_batch(InputFeatureSet::P, 3, 71).unwrap();
        let mut ring = HostRing::new(&device, InputFeatureSet::P, 3).unwrap();
        let mut timings = Vec::new();
        for _ in 0..5 {
            let ticket = ring.upload(&batch).unwrap();
            let delayed = ring.transfer_timing(&ticket).unwrap();
            let immediate = ring.transfer_timing(&ticket).unwrap().collect().unwrap();
            assert!(immediate.is_finite() && immediate >= 0.0);
            timings.push((delayed, immediate));
            ring.finish(ticket).unwrap();
        }
        for (delayed, immediate) in timings {
            // 時間の絶対値には依存せず、面の再利用が過去の測定を変えない契約を検査する。
            assert_eq!(delayed.collect().unwrap().to_bits(), immediate.to_bits());
        }
    }

    #[test]
    #[ignore = "requires a CUDA GPU"]
    fn gpu_ring_transfers_all_arrays_and_reuses_three_host_two_device_slots() {
        let device = Device::new(0).unwrap();
        for set in [InputFeatureSet::P, InputFeatureSet::HalfKpHmB8] {
            let mut ring = HostRing::new(&device, set, 100).unwrap();
            // 2面を同時に保持し、完了を記録してからだけ次の面へ進める。
            for iteration in 0..4 {
                let a = super::super::equivalence::synthetic_batch(
                    set,
                    100 - iteration,
                    iteration as u64,
                )
                .unwrap();
                let b = super::super::equivalence::synthetic_batch(
                    set,
                    1 + iteration,
                    42 + iteration as u64,
                )
                .unwrap();
                let ta = ring.upload(&a).unwrap();
                let tb = ring.upload(&b).unwrap();
                assert!(ring.upload(&a).is_err());
                for (ticket, batch) in [(ta, &a), (tb, &b)] {
                    let i = ring.input(&ticket).unwrap();
                    assert_eq!(i.len, batch.len());
                    assert_eq!(i.first_len, batch.first_layer_inverse().entries().len());
                    assert_eq!(i.linear_len, batch.linear_inverse().entries().len());
                    assert_eq!(
                        i.first_segments,
                        batch.first_layer_inverse().segment_count()
                    );
                    assert_eq!(i.linear_segments, batch.linear_inverse().segment_count());
                    macro_rules! matches {
                        ($field:ident, $expected:expr) => {
                            let expected = $expected;
                            let values = device
                                .compute
                                .clone_dtoh(&i.$field.slice(..expected.len()))
                                .unwrap();
                            assert_eq!(values, expected);
                        };
                    }
                    matches!(record_indices, batch.record_indices());
                    matches!(features, batch.features());
                    matches!(linear_features, batch.linear_features());
                    matches!(piece_counts, batch.piece_counts());
                    matches!(teacher_scores, batch.teacher_scores());
                    matches!(teacher_classes, batch.teacher_classes());
                    matches!(outcomes, batch.outcomes());
                    matches!(
                        first_segment_starts,
                        batch.first_layer_inverse().segment_starts()
                    );
                    matches!(
                        first_segment_offsets,
                        batch.first_layer_inverse().segment_offsets()
                    );
                    matches!(first_entries, batch.first_layer_inverse().entries());
                    matches!(
                        linear_segment_starts,
                        batch.linear_inverse().segment_starts()
                    );
                    matches!(
                        linear_segment_offsets,
                        batch.linear_inverse().segment_offsets()
                    );
                    matches!(linear_entries, batch.linear_inverse().entries());
                    ring.finish(ticket).unwrap();
                }
            }
        }
    }
    #[test]
    fn ring_requires_completion_and_rejects_stale_or_foreign_tickets() {
        let mut state = RingState::new();
        let a = state.reserve().unwrap();
        let b = state.reserve().unwrap();
        assert_eq!((a.host, a.device, b.host, b.device), (0, 0, 1, 1));
        assert!(state.reserve().is_err());
        assert!(RingState::new().check(&a).is_err());
        state.finish(&a).unwrap();
        assert!(state.check(&a).is_err());
        let c = state.reserve().unwrap();
        assert_eq!((c.host, c.device), (2, 0));
        assert!(state.finish(&a).is_err());
        state.finish(&b).unwrap();
        let d = state.reserve().unwrap();
        assert_eq!((d.host, d.device), (0, 1));
        state.poisoned = true;
        assert!(state.check(&c).is_err());
        assert!(state.reserve().is_err());
    }
}
