#![allow(non_upper_case_globals)]

use {
    crate::{
        async_engine::SpawnedFuture,
        format::{Format, pw_formats},
        pipewire::{
            pw_con::PwCon,
            pw_mem::{PwMemError, PwMemMap, PwMemSlice, PwMemTyped},
            pw_object::{PwObject, PwObjectData},
            pw_parser::{PwParser, PwParserError},
            pw_pod::{
                PW_CHOICE_Enum, PW_CHOICE_Flags, PW_NODE_ACTIVATION_FINISHED,
                PW_NODE_ACTIVATION_NOT_TRIGGERED, PW_NODE_ACTIVATION_TRIGGERED, PW_OBJECT_Format,
                PW_OBJECT_ParamBuffers, PW_OBJECT_ParamMeta, PW_PROP_DONT_FIXATE, PW_TYPE_Long,
                PwIoType, PwPod, PwPodFraction, PwPodObject, PwPodRectangle, PwPropFlag,
                SPA_DATA_DmaBuf, SPA_DATA_FLAG_READABLE, SPA_DATA_MemFd, SPA_DATA_MemPtr,
                SPA_DIRECTION_INPUT, SPA_DIRECTION_OUTPUT, SPA_FORMAT_VIDEO_format,
                SPA_FORMAT_VIDEO_framerate, SPA_FORMAT_VIDEO_modifier, SPA_FORMAT_VIDEO_size,
                SPA_FORMAT_mediaSubtype, SPA_FORMAT_mediaType, SPA_IO_Buffers, SPA_META_Bitmap,
                SPA_META_Busy, SPA_META_Control, SPA_META_Cursor, SPA_META_Header,
                SPA_META_VideoCrop, SPA_META_VideoDamage, SPA_NODE_BUFFERS_FLAG_ALLOC,
                SPA_NODE_COMMAND_Pause, SPA_NODE_COMMAND_Start, SPA_NODE_COMMAND_Suspend,
                SPA_PARAM_BUFFERS_blocks, SPA_PARAM_BUFFERS_buffers, SPA_PARAM_BUFFERS_dataType,
                SPA_PARAM_Buffers, SPA_PARAM_EnumFormat, SPA_PARAM_Format, SPA_PARAM_INFO,
                SPA_PARAM_INFO_READ, SPA_PARAM_INFO_SERIAL, SPA_PARAM_META_size,
                SPA_PARAM_META_type, SPA_PARAM_Meta, SPA_PORT_FLAG,
                SPA_PORT_FLAG_CAN_ALLOC_BUFFERS, SpaDataFlags, SpaDataType, SpaDirection,
                SpaIoType, SpaMediaSubtype, SpaMediaType, SpaMetaType, SpaNodeBuffersFlags,
                SpaNodeCommand, SpaParamType, SpaVideoFormat, pw_node_activation, spa_chunk,
                spa_io_buffers, spa_meta_bitmap, spa_meta_busy, spa_meta_cursor, spa_meta_header,
                spa_meta_region,
            },
        },
        utils::{
            bitfield::Bitfield, buf::TypedBuf, clonecell::CloneCell, copyhashmap::CopyHashMap,
            errorfmt::ErrorFmt, option_ext::OptionExt,
        },
        video::{Modifier, dmabuf::DmaBuf},
    },
    std::{
        cell::{Cell, RefCell},
        rc::Rc,
        sync::atomic::Ordering::{Relaxed, Release},
    },
    thiserror::Error,
    uapi::OwnedFd,
};

pw_opcodes! {
    PwClientNodeMethods;

    GetNode     = 1,
    Update      = 2,
    PortUpdate  = 3,
    SetActive   = 4,
    Event       = 5,
    PortBuffers = 6,
}

pw_opcodes! {
    PwClientNodeEvents;

    Transport      = 0,
    SetParam       = 1,
    SetIo          = 2,
    Event          = 3,
    Command        = 4,
    AddPort        = 5,
    RemovePort     = 6,
    PortSetParam   = 7,
    PortUseBuffers = 8,
    PortSetIo      = 9,
    SetActivation  = 10,
    PortSetMixInfo = 11,
}

pub trait PwClientNodeOwner {
    fn port_format_changed(&self, port: &Rc<PwClientNodePort>) {
        let _ = port;
    }
    fn use_buffers(self: Rc<Self>, port: &Rc<PwClientNodePort>) {
        let _ = port;
    }
    fn start(self: Rc<Self>) {}
    fn pause(self: Rc<Self>) {}
    fn suspend(self: Rc<Self>) {}
    fn bound_id(&self, id: u32) {
        let _ = id;
    }
}

bitflags! {
    PwClientNodePortChanges: u32;

    CHANGED_SUPPORTED_PARAMS = 1 << 0,
}

bitflags! {
    PwClientNodePortSupportedMetas: u32;

    SUPPORTED_META_HEADER = 1 << 0,
    SUPPORTED_META_BUSY = 1 << 1,
    SUPPORTED_META_VIDEO_CROP = 1 << 2,
}

pub struct PwClientNodePort {
    pub node: Rc<PwClientNode>,

    pub direction: SpaDirection,
    pub id: u32,

    pub _destroyed: Cell<bool>,

    pub negotiated_format: RefCell<PwClientNodePortFormat>,
    pub supported_formats: RefCell<PwClientNodePortSupportedFormats>,
    pub supported_metas: Cell<PwClientNodePortSupportedMetas>,
    pub can_alloc_buffers: Cell<bool>,

    pub buffers: RefCell<Vec<Rc<PwClientNodeBuffer>>>,

    pub buffer_config: RefCell<PwClientNodeBufferConfig>,

    pub io_buffers: CloneCell<Option<Rc<PwMemTyped<spa_io_buffers>>>>,

    pub serial: Cell<bool>,
}

#[derive(Copy, Clone, Debug, Default)]
pub struct PwClientNodeBufferConfig {
    pub num_buffers: Option<usize>,
    pub planes: Option<usize>,
    pub data_type: SpaDataType,
}

pub struct PwClientNodeBuffer {
    pub _meta_header: Option<Rc<PwMemTyped<spa_meta_header>>>,
    pub _meta_busy: Option<Rc<PwMemTyped<spa_meta_busy>>>,
    pub meta_video_crop: Option<Rc<PwMemTyped<spa_meta_region>>>,
    pub chunks: Vec<Rc<PwMemTyped<spa_chunk>>>,
    pub _slices: Vec<Rc<PwMemSlice>>,
}

#[derive(Clone, Debug)]
pub struct PwClientNodePortSupportedFormat {
    pub format: &'static Format,
    pub modifiers: Vec<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct PwClientNodePortSupportedFormats {
    pub media_type: Option<SpaMediaType>,
    pub media_sub_type: Option<SpaMediaSubtype>,
    pub video_size: Option<PwPodRectangle>,
    pub formats: Vec<PwClientNodePortSupportedFormat>,
}

#[derive(Clone, Debug, Default)]
pub struct PwClientNodePortFormat {
    pub media_type: Option<SpaMediaType>,
    pub media_sub_type: Option<SpaMediaSubtype>,
    pub video_size: Option<PwPodRectangle>,
    pub format: Option<&'static Format>,
    pub modifiers: Option<Vec<Modifier>>,
    pub framerate: Option<PwPodFraction>,
}

pub struct PwClientNode {
    pub data: PwObjectData,
    pub con: Rc<PwCon>,
    pub ios: CopyHashMap<PwIoType, Rc<PwMemMap>>,

    pub owner: CloneCell<Option<Rc<dyn PwClientNodeOwner>>>,

    pub ports: CopyHashMap<(SpaDirection, u32), Rc<PwClientNodePort>>,

    pub port_out_free: RefCell<Bitfield>,
    pub port_in_free: RefCell<Bitfield>,

    pub activation: CloneCell<Option<Rc<PwMemTyped<pw_node_activation>>>>,
    pub transport_in: Cell<Option<SpawnedFuture<()>>>,
    pub transport_out: CloneCell<Option<Rc<OwnedFd>>>,

    pub activations: CopyHashMap<u32, Rc<PwNodeActivation>>,
}

pub struct PwNodeActivation {
    pub activation: Rc<PwMemTyped<pw_node_activation>>,
    pub fd: Rc<OwnedFd>,
}

// pub struct PwNodeBuffer {
//     pub width: i32,
//     pub height: i32,
//     pub stride: i32,
//     pub offset: i32,
//     pub fd: Rc<OwnedFd>,
// }

pub const PW_CLIENT_NODE_FACTORY: &str = "client-node";
pub const PW_CLIENT_NODE_INTERFACE: &str = "PipeWire:Interface:ClientNode";
pub const PW_CLIENT_NODE_VERSION: i32 = 4;

#[expect(dead_code)]
const PW_CLIENT_NODE_UPDATE_PARAMS: u32 = 1 << 0;
const PW_CLIENT_NODE_UPDATE_INFO: u32 = 1 << 1;

const SPA_NODE_CHANGE_MASK_FLAGS: u64 = 1 << 0;
#[expect(dead_code)]
const SPA_NODE_CHANGE_MASK_PROPS: u64 = 1 << 1;
const SPA_NODE_CHANGE_MASK_PARAMS: u64 = 1 << 2;

const PW_CLIENT_NODE_PORT_UPDATE_PARAMS: u32 = 1 << 0;
const PW_CLIENT_NODE_PORT_UPDATE_INFO: u32 = 1 << 1;

const SPA_PORT_CHANGE_MASK_FLAGS: u64 = 1 << 0;
const SPA_PORT_CHANGE_MASK_RATE: u64 = 1 << 1;
#[expect(dead_code)]
const SPA_PORT_CHANGE_MASK_PROPS: u64 = 1 << 2;
const SPA_PORT_CHANGE_MASK_PARAMS: u64 = 1 << 3;

impl PwClientNode {
    pub fn send_update(&self) {
        self.con.send(self, PwClientNodeMethods::Update, |f| {
            f.write_struct(|f| {
                f.write_uint(PW_CLIENT_NODE_UPDATE_INFO);
                f.write_uint(0);
                f.write_struct(|f| {
                    f.write_uint(0);
                    f.write_uint(1);
                    f.write_ulong(SPA_NODE_CHANGE_MASK_PARAMS | SPA_NODE_CHANGE_MASK_FLAGS);
                    f.write_ulong(0);
                    f.write_uint(0);
                    f.write_uint(0);
                });
            });
        });
    }

    pub fn send_active(&self, active: bool) {
        self.con.send(self, PwClientNodeMethods::SetActive, |f| {
            f.write_struct(|f| {
                f.write_bool(active);
            });
        });
    }

    pub fn create_port(
        self: &Rc<Self>,
        output: bool,
        supported_formats: PwClientNodePortSupportedFormats,
        num_buffers: Option<usize>,
    ) -> Rc<PwClientNodePort> {
        let (ids, direction) = match output {
            true => (&self.port_out_free, SPA_DIRECTION_OUTPUT),
            false => (&self.port_in_free, SPA_DIRECTION_INPUT),
        };
        let port = Rc::new(PwClientNodePort {
            node: self.clone(),
            direction,
            id: ids.borrow_mut().acquire(),
            _destroyed: Cell::new(false),
            negotiated_format: Default::default(),
            supported_formats: RefCell::new(supported_formats),
            supported_metas: Cell::new(PwClientNodePortSupportedMetas::none()),
            can_alloc_buffers: Cell::new(false),
            buffers: RefCell::new(vec![]),
            buffer_config: RefCell::new(PwClientNodeBufferConfig {
                num_buffers,
                planes: None,
                data_type: SPA_DATA_DmaBuf,
            }),
            io_buffers: Default::default(),
            serial: Cell::new(false),
        });
        self.ports.set((direction, port.id), port.clone());
        port
    }

    pub fn send_port_output_buffers(&self, port: &PwClientNodePort, buffers: &[DmaBuf]) {
        self.con.send(self, PwClientNodeMethods::PortBuffers, |f| {
            f.write_struct(|f| {
                // direction
                f.write_uint(port.direction.0);
                // id
                f.write_uint(port.id);
                // mix_id
                f.write_int(-1);
                // n_buffers
                f.write_uint(buffers.len() as _);
                for buffer in buffers {
                    // n_datas
                    f.write_uint(buffer.planes.len() as _);
                    for plane in &buffer.planes {
                        // type
                        f.write_id(SPA_DATA_DmaBuf.0);
                        // fd
                        f.write_fd(&plane.fd);
                        // flags
                        f.write_uint(SPA_DATA_FLAG_READABLE.0);
                        // offset
                        f.write_uint(plane.offset);
                        // size
                        f.write_uint(plane.stride * buffer.height as u32);
                    }
                }
            });
        });
    }

    pub fn send_port_update(&self, port: &PwClientNodePort, fixate: bool) {
        port.serial.set(!port.serial.get());
        let serial = match port.serial.get() {
            true => SPA_PARAM_INFO_SERIAL,
            false => SPA_PARAM_INFO::none(),
        };
        self.con.send(self, PwClientNodeMethods::PortUpdate, |f| {
            f.write_struct(|f| {
                // direction
                f.write_uint(port.direction.0);
                // id
                f.write_uint(port.id);
                // change flags
                f.write_uint(PW_CLIENT_NODE_PORT_UPDATE_PARAMS | PW_CLIENT_NODE_PORT_UPDATE_INFO);
                let sm = port.supported_metas.get();
                let mut metas = vec![];
                if sm.contains(SUPPORTED_META_HEADER) {
                    metas.push((SPA_META_Header, size_of::<spa_meta_header>()));
                }
                if sm.contains(SUPPORTED_META_BUSY) {
                    metas.push((SPA_META_Busy, size_of::<spa_meta_busy>()));
                }
                if sm.contains(SUPPORTED_META_VIDEO_CROP) {
                    metas.push((SPA_META_VideoCrop, size_of::<spa_meta_region>()));
                }
                let sf = &*port.supported_formats.borrow();
                let num_formats = sf.formats.len() as u32;
                let bc = &*port.buffer_config.borrow();
                let num_params = metas.len() as u32 + num_formats + 1;

                // num params
                f.write_uint(num_params);
                for format in &sf.formats {
                    f.write_object(PW_OBJECT_Format, SPA_PARAM_EnumFormat.0, |f| {
                        if let Some(mt) = sf.media_type {
                            f.write_property(SPA_FORMAT_mediaType.0, PwPropFlag::none(), |f| {
                                f.write_id(mt.0);
                            });
                        }
                        if let Some(mst) = sf.media_sub_type {
                            f.write_property(SPA_FORMAT_mediaSubtype.0, PwPropFlag::none(), |f| {
                                f.write_id(mst.0);
                            });
                        }
                        f.write_property(SPA_FORMAT_VIDEO_format.0, PwPropFlag::none(), |f| {
                            f.write_choice(PW_CHOICE_Enum, 0, |f| {
                                f.write_id(format.format.pipewire.0);
                                f.write_id(format.format.pipewire.0);
                            });
                        });
                        f.write_property(
                            SPA_FORMAT_VIDEO_modifier.0,
                            if fixate {
                                PwPropFlag::none()
                            } else {
                                PW_PROP_DONT_FIXATE
                            },
                            |f| {
                                f.write_choice(PW_CHOICE_Enum, 0, |f| {
                                    f.write_ulong(format.modifiers[0]);
                                    for modifier in &format.modifiers {
                                        f.write_ulong(*modifier);
                                    }
                                });
                            },
                        );
                        if let Some(vs) = sf.video_size {
                            f.write_property(SPA_FORMAT_VIDEO_size.0, PwPropFlag::none(), |f| {
                                f.write_choice(PW_CHOICE_Enum, 0, |f| {
                                    f.write_rectangle(vs.width, vs.height);
                                    f.write_rectangle(vs.width, vs.height);
                                });
                            });
                        }
                    });
                }
                f.write_object(PW_OBJECT_ParamBuffers, SPA_PARAM_Buffers.0, |f| {
                    if let Some(num_buffers) = bc.num_buffers {
                        f.write_property(SPA_PARAM_BUFFERS_buffers.0, PwPropFlag::none(), |f| {
                            f.write_uint(num_buffers as _);
                        });
                    }
                    if let Some(planes) = bc.planes {
                        f.write_property(SPA_PARAM_BUFFERS_blocks.0, PwPropFlag::none(), |f| {
                            f.write_uint(planes as _);
                        });
                    }
                    f.write_property(SPA_PARAM_BUFFERS_dataType.0, PwPropFlag::none(), |f| {
                        f.write_choice(PW_CHOICE_Flags, 0, |f| {
                            f.write_uint(1 << bc.data_type.0);
                        });
                    });
                });
                for (key, size) in metas {
                    f.write_object(PW_OBJECT_ParamMeta, SPA_PARAM_Meta.0, |f| {
                        f.write_property(SPA_PARAM_META_type.0, PwPropFlag::none(), |f| {
                            f.write_id(key.0);
                        });
                        f.write_property(SPA_PARAM_META_size.0, PwPropFlag::none(), |f| {
                            f.write_uint(size as u32);
                        });
                    });
                }
                f.write_struct(|f| {
                    // change mask
                    f.write_ulong(
                        SPA_PORT_CHANGE_MASK_FLAGS
                            // | SPA_PORT_CHANGE_MASK_PROPS
                            | SPA_PORT_CHANGE_MASK_PARAMS
                            | SPA_PORT_CHANGE_MASK_RATE,
                    );
                    let mut flags = SPA_PORT_FLAG::none();
                    if port.can_alloc_buffers.get() {
                        flags = SPA_PORT_FLAG_CAN_ALLOC_BUFFERS;
                    }
                    // flags
                    f.write_ulong(flags.0);
                    // rate num
                    f.write_int(0);
                    // rate denom
                    f.write_int(1);
                    // num props
                    f.write_int(0);
                    let num_params = 3;
                    // num params
                    f.write_uint(num_params);
                    f.write_id(SPA_PARAM_EnumFormat.0);
                    f.write_uint((SPA_PARAM_INFO_READ | serial).0);
                    f.write_id(SPA_PARAM_Buffers.0);
                    f.write_uint((SPA_PARAM_INFO_READ | serial).0);
                    f.write_id(SPA_PARAM_Meta.0);
                    f.write_uint(SPA_PARAM_INFO_READ.0);
                });
            });
        });
    }

    fn handle_set_param(&self, _p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        Ok(())
    }

    fn handle_set_io(&self, mut p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        let s = p.read_struct()?;
        let mut p2 = s.fields;
        let id = PwIoType(p2.read_id()?);
        let memid = p2.read_uint()?;
        let offset = p2.read_uint()?;
        let size = p2.read_uint()?;
        log::debug!("set io {:?}", id);
        if memid == !0 {
            self.ios.remove(&id);
        } else {
            let map = match self.con.mem.map(memid, offset, size) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Could not map memory from the pool: {}", ErrorFmt(e));
                    return Ok(());
                }
            };
            self.ios.set(id, map);
        }
        Ok(())
    }

    fn handle_event(&self, _p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        Ok(())
    }

    fn handle_command(self: &Rc<Self>, mut p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        let s1 = p.read_struct()?;
        let mut p1 = s1.fields;
        let obj = p1.read_object()?;
        match SpaNodeCommand(obj.id) {
            SPA_NODE_COMMAND_Start => {
                if let Some(owner) = self.owner.get() {
                    owner.start();
                }
            }
            SPA_NODE_COMMAND_Pause => {
                if let Some(owner) = self.owner.get() {
                    owner.pause();
                }
            }
            SPA_NODE_COMMAND_Suspend => {
                if let Some(owner) = self.owner.get() {
                    owner.suspend();
                }
            }
            v => {
                log::warn!("Unhandled node command {:?}", v);
            }
        }
        Ok(())
    }

    fn handle_add_port(&self, _p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        Ok(())
    }

    fn handle_remove_port(&self, _p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        Ok(())
    }

    fn port_set_format(
        &self,
        port: &Rc<PwClientNodePort>,
        obj: Option<PwPodObject<'_>>,
    ) -> Result<(), PwClientNodeError> {
        let mut obj = match obj {
            Some(obj) => obj,
            _ => {
                port.negotiated_format.take();
                return Ok(());
            }
        };
        let mut format = PwClientNodePortFormat::default();
        if let Some(mt) = obj.get_param(SPA_FORMAT_mediaType.0)? {
            format.media_type = Some(SpaMediaType(mt.pod.get_id()?));
        }
        if let Some(mt) = obj.get_param(SPA_FORMAT_mediaSubtype.0)? {
            format.media_sub_type = Some(SpaMediaSubtype(mt.pod.get_id()?));
        }
        if let Some(mt) = obj.get_param(SPA_FORMAT_VIDEO_size.0)? {
            format.video_size = Some(mt.pod.get_rectangle()?);
        }
        if let Some(mt) = obj.get_param(SPA_FORMAT_VIDEO_format.0)?
            && let Some(fmt) = pw_formats().get(&SpaVideoFormat(mt.pod.get_id()?))
        {
            format.format = Some(*fmt);
        }
        if let Some(mt) = obj.get_param(SPA_FORMAT_VIDEO_modifier.0)?
            && let PwPod::Choice(mods) = mt.pod
        {
            let mut p1 = mods.elements.elements;
            p1.read_pod_body_packed(PW_TYPE_Long, 8)?;
            while p1.len() > 0 {
                let modifier = p1.read_pod_body_packed(PW_TYPE_Long, 8)?;
                if let PwPod::Long(modifier) = modifier {
                    format
                        .modifiers
                        .get_or_insert_default_ext()
                        .push(modifier as u64);
                }
            }
        }
        if let Some(mt) = obj.get_param(SPA_FORMAT_VIDEO_framerate.0)? {
            format.framerate = Some(mt.pod.get_fraction()?);
        }
        *port.negotiated_format.borrow_mut() = format;
        Ok(())
    }

    fn handle_port_set_param(&self, mut p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        let s1 = p.read_struct()?;
        let mut p1 = s1.fields;
        let direction = SpaDirection(p1.read_uint()?);
        let port_id = p1.read_uint()?;
        let id = SpaParamType(p1.read_id()?);
        let _flags = p1.read_int()?;
        let obj = p1.read_object_opt()?;
        let port = self.get_port(direction, port_id)?;
        match id {
            SPA_PARAM_Format => {
                self.port_set_format(&port, obj)?;
                if let Some(owner) = self.owner.get() {
                    owner.port_format_changed(&port);
                }
            }
            _ => {
                log::warn!(
                    "port_set_param: Ignoring unexpected port parameter {:?}",
                    id
                );
            }
        }
        Ok(())
    }

    fn handle_port_use_buffers(&self, mut p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        let s1 = p.read_struct()?;
        let mut p1 = s1.fields;
        let direction = SpaDirection(p1.read_uint()?);
        let port_id = p1.read_uint()?;
        let _mix_id = p1.read_int()?;
        let buffer_flags = SpaNodeBuffersFlags(p1.read_uint()?);
        let n_buffers = p1.read_uint()?;
        let port = self.get_port(direction, port_id)?;

        let mut res = vec![];

        for _ in 0..n_buffers {
            let mem_id = p1.read_uint()?;
            let offset = p1.read_uint()?;
            let size = p1.read_uint()?;
            let n_metas = p1.read_uint()?;

            let mut meta_header = Default::default();
            let mut meta_video_crop = Default::default();
            let mut meta_busy = Default::default();
            let mut chunks = vec![];
            let mut slices = vec![];

            let mem = self.con.mem.map(mem_id, offset, size)?;

            log::debug!("  mem_id={}, offset={}, size={}", mem_id, offset, size);
            log::debug!("  n_metas={}", n_metas);

            let mut offset = 0;

            for _ in 0..n_metas {
                let ty = SpaMetaType(p1.read_id()?);
                let size = p1.read_uint()? as usize;

                match ty {
                    SPA_META_Header => {
                        let header = mem.typed_at::<spa_meta_header>(offset);
                        meta_header = Some(header);
                    }
                    SPA_META_VideoCrop => {
                        let crop = mem.typed_at::<spa_meta_region>(offset);
                        meta_video_crop = Some(crop);
                    }
                    SPA_META_VideoDamage => {
                        let _video_damage = mem.typed_at::<spa_meta_region>(offset);
                    }
                    SPA_META_Bitmap => {
                        let _bitmap = mem.typed_at::<spa_meta_bitmap>(offset);
                    }
                    SPA_META_Cursor => {
                        let _cursor = mem.typed_at::<spa_meta_cursor>(offset);
                    }
                    SPA_META_Control => {}
                    SPA_META_Busy => {
                        let busy = mem.typed_at::<spa_meta_busy>(offset);
                        meta_busy = Some(busy);
                    }
                    _ => {}
                }

                offset += (size + 7) & !7;
            }

            let n_datas = p1.read_uint()?;

            log::debug!("  offset = {}, n_datas={}", offset, n_datas);

            for _ in 0..n_datas {
                let ty = SpaDataType(p1.read_id()?);
                let data_id = p1.read_uint()?;
                let _flags = SpaDataFlags(p1.read_uint()?);
                let mapoffset = p1.read_uint()?;
                let maxsize = p1.read_uint()?;

                chunks.push(mem.typed_at(offset));
                offset += size_of::<spa_chunk>();

                if !buffer_flags.contains(SPA_NODE_BUFFERS_FLAG_ALLOC) {
                    if ty == SPA_DATA_MemPtr {
                        let offset = data_id as usize;
                        slices.push(mem.slice(offset..offset + maxsize as usize));
                    } else if ty == SPA_DATA_MemFd {
                        let mem = self.con.mem.map(data_id, mapoffset, maxsize)?;
                        slices.push(mem.slice(0..maxsize as usize));
                    }
                }
            }

            res.push(Rc::new(PwClientNodeBuffer {
                _meta_header: meta_header,
                _meta_busy: meta_busy,
                meta_video_crop,
                chunks,
                _slices: slices,
            }));
        }

        *port.buffers.borrow_mut() = res;

        if let Some(owner) = self.owner.get() {
            owner.use_buffers(&port);
        }

        Ok(())
    }

    fn handle_port_set_io(&self, mut p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        let s = p.read_struct()?;
        let mut p2 = s.fields;
        let direction = SpaDirection(p2.read_uint()?);
        let port_id = p2.read_uint()?;
        let mix_id = p2.read_uint()?;
        let id = SpaIoType(p2.read_id()?);
        let mem_id = p2.read_uint()?;
        let offset = p2.read_uint()?;
        let size = p2.read_uint()?;
        let port = self.get_port(direction, port_id)?;
        match id {
            SPA_IO_Buffers if mix_id == 0 => {
                if mem_id == !0 {
                    port.io_buffers.take();
                } else {
                    let io_buffers = self
                        .con
                        .mem
                        .map(mem_id, offset, size)?
                        .typed::<spa_io_buffers>();
                    unsafe {
                        io_buffers.read().buffer_id.store(!0, Relaxed);
                        io_buffers.read().status.store(0, Relaxed);
                    }
                    port.io_buffers.set(Some(io_buffers));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_transport(self: &Rc<Self>, mut p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        let s = p.read_struct()?;
        let mut p2 = s.fields;
        let readfd = p2.read_fd()?;
        let writefd = p2.read_fd()?;
        let memid = p2.read_uint()?;
        let offset = p2.read_uint()?;
        let size = p2.read_uint()?;
        let map = match self.con.mem.map(memid, offset, size) {
            Ok(m) => m,
            Err(e) => {
                log::error!("Could not map memory from the pool: {}", ErrorFmt(e));
                return Ok(());
            }
        };
        let typed = map.typed::<pw_node_activation>();
        self.activation.set(Some(typed.clone()));
        self.transport_in.set(Some(
            self.con
                .eng
                .spawn("pw transport in", self.clone().transport_in(typed, readfd)),
        ));
        self.transport_out.set(Some(writefd));
        Ok(())
    }

    fn handle_set_activation(
        self: &Rc<Self>,
        mut p: PwParser<'_>,
    ) -> Result<(), PwClientNodeError> {
        let s = p.read_struct()?;
        let mut p2 = s.fields;
        let node = p2.read_uint()?;
        let signalfd = p2.read_fd_opt()?;
        if let Some(signalfd) = signalfd {
            let memid = p2.read_uint()?;
            let offset = p2.read_uint()?;
            let size = p2.read_uint()?;
            let map = match self.con.mem.map(memid, offset, size) {
                Ok(m) => m,
                Err(e) => {
                    log::error!("Could not map memory from the pool: {}", ErrorFmt(e));
                    return Ok(());
                }
            };
            let typed = map.typed::<pw_node_activation>();
            self.activations.set(
                node,
                Rc::new(PwNodeActivation {
                    activation: typed,
                    fd: signalfd,
                }),
            );
        } else {
            self.activations.remove(&node);
        }
        Ok(())
    }

    fn get_port(
        &self,
        direction: SpaDirection,
        port_id: u32,
    ) -> Result<Rc<PwClientNodePort>, PwClientNodeError> {
        match self.ports.get(&(direction, port_id)) {
            Some(p) => Ok(p),
            _ => Err(PwClientNodeError::UnknownPort(direction, port_id)),
        }
    }

    fn handle_port_set_mix_info(&self, mut p: PwParser<'_>) -> Result<(), PwClientNodeError> {
        let s1 = p.read_struct()?;
        let mut p1 = s1.fields;
        let direction = SpaDirection(p1.read_uint()?);
        let port_id = p1.read_uint()?;
        let mix_id = p1.read_int()?;
        let peer_id = p1.read_int()?;
        let dict = p1.read_dict_struct()?;
        let _port = self.get_port(direction, port_id)?;
        log::debug!(
            "mix info: mix_id={}, peer_id={}, dict={:#?}",
            mix_id,
            peer_id,
            dict
        );
        Ok(())
    }

    async fn transport_in(
        self: Rc<Self>,
        _activation: Rc<PwMemTyped<pw_node_activation>>,
        fd: Rc<OwnedFd>,
    ) {
        let mut buf = TypedBuf::<u64>::new();
        loop {
            if let Err(e) = self.con.ring.read(&fd, buf.buf()).await {
                log::error!("Could not read from eventfd: {}", ErrorFmt(e));
                return;
            }
            if let Some(activation) = self.activation.get() {
                let activation = unsafe { activation.read() };
                activation
                    .status
                    .store(PW_NODE_ACTIVATION_FINISHED.0, Relaxed);
            }
        }
    }

    pub fn drive(&self) {
        for activation in self.activations.lock().values() {
            let a = unsafe { activation.activation.read() };
            let required = a.state[0].required.load(Relaxed);
            a.state[0].pending.store(required - 1, Relaxed);
            if required == 1 {
                a.status.store(PW_NODE_ACTIVATION_TRIGGERED.0, Release);
                let _ = uapi::eventfd_write(activation.fd.raw(), 1);
            } else {
                a.status.store(PW_NODE_ACTIVATION_NOT_TRIGGERED.0, Release);
            }
        }
    }
}

pw_object_base! {
    PwClientNode, "client-node", PwClientNodeEvents;

    Transport      => handle_transport,
    SetParam       => handle_set_param,
    SetIo          => handle_set_io,
    Event          => handle_event,
    Command        => handle_command,
    AddPort        => handle_add_port,
    RemovePort     => handle_remove_port,
    PortSetParam   => handle_port_set_param,
    PortUseBuffers => handle_port_use_buffers,
    PortSetIo      => handle_port_set_io,
    SetActivation  => handle_set_activation,
    PortSetMixInfo => handle_port_set_mix_info,
}

impl PwObject for PwClientNode {
    fn bound_id(&self, id: u32) {
        if let Some(owner) = self.owner.get() {
            owner.bound_id(id);
        }
    }

    fn break_loops(&self) {
        self.owner.take();
        self.ports.clear();
        self.transport_in.take();
        self.transport_out.take();
    }
}

#[derive(Debug, Error)]
pub enum PwClientNodeError {
    #[error(transparent)]
    PwParserError(#[from] PwParserError),
    #[error(transparent)]
    PwMemError(#[from] PwMemError),
    #[error("Unknown port {0:?}@{1}")]
    UnknownPort(SpaDirection, u32),
}
