use core::ops::Range;

use alloc::{collections::BTreeMap, sync::Arc, vec::Vec};
use bitflags::bitflags;
use lazy_static::lazy_static;
use riscv::register::satp;
use xmas_elf::{program, ElfFile};

use crate::{
    config::{MEMORY_END, MMIO, PAGE_SIZE, TRAMPOLINE, TRAP_CONTEXT, USER_STACK_SIZE},
    sync::UPSafeCell,
};

use super::{
    address::{PhysAddr, PhysPageNum, VirtAddr, VirtPageNum},
    frame_allocator::{frame_alloc, FrameTracker},
    page_table::{PTEFlags, PageTable, PageTableEntry},
};

bitflags! {
    /// 控制一个逻辑段的访问方式。是 `PTEFlags` 的严格子集。
    ///
    /// 包括 R/W/X 和 U
    pub struct MapPermission: u8 {
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
    }
}

lazy_static! {
    pub static ref KERNEL_SPACE: Arc<UPSafeCell<MemorySet>> =
        Arc::new(unsafe { UPSafeCell::new(MemorySet::new_kernel()) });
}

/// 用于描述逻辑上连续的虚拟内存段。
///
/// 段中的每一页都具有相同的 flag。
///
/// 内核自己的代码、数据等以 `Identical` 方式映射
///
/// 而 APP 以及内核中和 APP 相关的
#[derive(Debug)]
pub struct MapArea {
    pub vpn_range: Range<VirtPageNum>,
    map_type: MapType,
    map_perm: MapPermission,
}

/// 描述逻辑段内所有虚拟页映射到物理页的方式
pub enum MapType {
    /// 恒等映射，或者说直接以物理地址访问
    Identical,
    /// 需要分配物理页帧
    Framed {
        /// 这些保存的物理页帧用于存放实际的内存数据
        ///
        /// 而 PageTable 所拥有的的物理页仅用于存放页表节点数据，因此不会冲突
        data_frames: BTreeMap<VirtPageNum, FrameTracker>,
    },
}

impl core::fmt::Debug for MapType {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MapType::Identical => write!(f, "MapType::Identical"),
            MapType::Framed { data_frames: _ } => write!(f, "MapType::Framed"),
        }
    }
}

impl MapArea {
    pub fn new(
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_type: MapType,
        map_perm: MapPermission,
    ) -> Self {
        let start_vpn = start_va.floor();
        let end_va = end_va.ceil();
        Self {
            vpn_range: start_vpn..end_va,
            map_type,
            map_perm,
        }
    }
    pub fn from_another(another: &MapArea) -> Self {
        Self {
            vpn_range: another.vpn_range.clone(),
            map_type: match another.map_type {
                MapType::Identical => MapType::Identical,
                MapType::Framed { .. } => MapType::Framed {
                    data_frames: BTreeMap::new(),
                },
            },
            map_perm: another.map_perm,
        }
    }
    // 在 `page_table` 中将本逻辑段映射
    pub fn map(&mut self, page_table: &mut PageTable) {
        log::trace!(
            "{}:{}, vpn_range: {:#x}~{:#x}",
            file!(),
            line!(),
            self.vpn_range.start.0,
            self.vpn_range.end.0
        );
        for vpn in self.vpn_range.clone() {
            self.map_one(page_table, vpn);
        }
    }
    // 在 `page_table` 中将本逻辑段解除映射
    pub fn unmap(&mut self, page_table: &mut PageTable) {
        for vpn in self.vpn_range.clone() {
            self.unmap_one(page_table, vpn);
        }
    }
    /// 约定：当前逻辑段必须是 `Framed` 的。而且 `data` 的长度不得超过逻辑段长度。
    pub fn copy_data(&mut self, page_table: &mut PageTable, data: &[u8]) {
        let mut curr_vpn = self.vpn_range.start;
        for chunk in data.chunks(PAGE_SIZE) {
            let mut dst = page_table.translate(curr_vpn).unwrap().ppn();
            dst.copy_from(chunk);
            curr_vpn.0 += 1;
        }
    }
    pub fn map_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        let ppn;
        match &mut self.map_type {
            MapType::Identical => ppn = PhysPageNum(vpn.0),
            MapType::Framed { data_frames } => {
                let frame = frame_alloc().expect("Should have enough memory");
                ppn = frame.ppn;
                data_frames.insert(vpn, frame);
            }
        };
        page_table.map(vpn, ppn, PTEFlags::from_bits_truncate(self.map_perm.bits));
    }
    pub fn unmap_one(&mut self, page_table: &mut PageTable, vpn: VirtPageNum) {
        if let MapType::Framed { data_frames } = &mut self.map_type {
            data_frames.remove(&vpn);
        }
        page_table.unmap(vpn);
    }

    /// 判断 `r` 是否与本段相交——前提是 `r` 是一个有效的范围
    pub fn intersection(&self, r: &Range<VirtPageNum>) -> Range<VirtPageNum> {
        self.vpn_range.start.max(r.start)..self.vpn_range.end.min(r.end)
    }
}

/// 地址空间是一系列有关联的逻辑段，这些逻辑段一般属于同一个进程
#[derive(Debug)]
pub struct MemorySet {
    pub page_table: PageTable,
    pub areas: Vec<MapArea>,
}

extern "C" {
    fn stext();
    fn etext();
    fn srodata();
    fn erodata();
    fn sdata();
    fn edata();
    fn sbss_with_stack();
    fn ebss();
    fn ekernel();
    fn strampoline();
}

impl MemorySet {
    pub fn new_bare() -> Self {
        Self {
            page_table: PageTable::new(),
            areas: Vec::new(),
        }
    }
    pub fn from_existed_user(user_space: &MemorySet) -> Self {
        let mut memory_set = Self::new_bare();
        memory_set.map_trampoline();
        for area in &user_space.areas {
            let new_area = MapArea::from_another(area);
            memory_set.push(new_area, None);
            for vpn in area.vpn_range.clone() {
                let src_ppn = user_space.translate(vpn).unwrap().ppn();
                let mut dst_ppn = memory_set.translate(vpn).unwrap().ppn();
                dst_ppn
                    .as_page_bytes_mut()
                    .copy_from_slice(src_ppn.as_page_bytes());
            }
        }
        memory_set
    }
    // 启动虚拟内存机制
    pub fn activate(&self) {
        let satp = self.page_table.satp();
        satp::write(satp);
        unsafe {
            // 清空 TLB
            core::arch::asm!("sfence.vma");
        }
    }
    pub fn remove_area_with_start_vpn(&mut self, start_vpn: VirtPageNum) {
        if let Some((idx, area)) = self
            .areas
            .iter_mut()
            .enumerate()
            .find(|(_, area)| area.vpn_range.start == start_vpn)
        {
            area.unmap(&mut self.page_table);
            self.areas.swap_remove(idx);
        }
    }
    fn push(&mut self, mut map_area: MapArea, data: Option<&[u8]>) {
        map_area.map(&mut self.page_table);
        if let Some(data) = data {
            map_area.copy_data(&mut self.page_table, data);
        }
        self.areas.push(map_area);
    }
    /// 在当前地址空间插入一个 `Framed` 方式映射的逻辑段。需要保证同一地址空间内的两个逻辑段不能相交
    pub fn insert_framed_area(
        &mut self,
        start_va: VirtAddr,
        end_va: VirtAddr,
        map_perm: MapPermission,
    ) {
        self.push(
            MapArea::new(
                start_va,
                end_va,
                MapType::Framed {
                    data_frames: Default::default(),
                },
                map_perm,
            ),
            None,
        );
    }
    pub fn recycle_data_pages(&mut self) {
        self.areas.clear();
    }
    /// 生成内核的地址空间
    pub fn new_kernel() -> Self {
        let mut memory_set = Self::new_bare();
        // map trampoline
        memory_set.map_trampoline();
        // map kernel sections
        log::info!(".text [{:#x}, {:#x})", stext as usize, etext as usize);
        log::info!(".rodata [{:#x}, {:#x})", srodata as usize, erodata as usize);
        log::info!(".data [{:#x}, {:#x})", sdata as usize, edata as usize);
        log::info!(
            ".bss [{:#x}, {:#x})",
            sbss_with_stack as usize,
            ebss as usize
        );
        log::info!("mapping .text section");
        memory_set.push(
            MapArea::new(
                VirtAddr(stext as usize),
                VirtAddr(etext as usize),
                MapType::Identical,
                MapPermission::R | MapPermission::X,
            ),
            None,
        );
        log::info!("mapping .rodata section");
        memory_set.push(
            MapArea::new(
                VirtAddr(srodata as usize),
                VirtAddr(erodata as usize),
                MapType::Identical,
                MapPermission::R,
            ),
            None,
        );
        log::info!("mapping .data section");
        memory_set.push(
            MapArea::new(
                VirtAddr(sdata as usize),
                VirtAddr(edata as usize),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        log::info!("mapping .bss section");
        memory_set.push(
            MapArea::new(
                VirtAddr(sbss_with_stack as usize),
                VirtAddr(ebss as usize),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        log::info!("mapping physical memory");
        memory_set.push(
            MapArea::new(
                VirtAddr(ekernel as usize),
                VirtAddr(MEMORY_END),
                MapType::Identical,
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        log::info!("mapping memory-mapped registers");
        for pair in MMIO {
            memory_set.push(
                MapArea::new(
                    VirtAddr(pair.0),
                    VirtAddr(pair.0 + pair.1),
                    MapType::Identical,
                    MapPermission::R | MapPermission::W,
                ),
                None,
            )
        }
        memory_set
    }
    /// 从 ELF 数据中解析出各类数据段并对应生成应用的地址空间、用户栈和入口
    ///
    /// 返回 (memory_set, user_stack_top, entry)
    pub fn from_elf(elf_data: &[u8]) -> (Self, usize, usize) {
        let mut memory_set = Self::new_bare();
        memory_set.map_trampoline();
        let elf = ElfFile::new(elf_data).unwrap();
        let elf_header = elf.header;
        let magic = elf_header.pt1.magic;
        assert_eq!(magic, [0x7f, 0x45, 0x4c, 0x46], "invalid elf!");
        let ph_count = elf_header.pt2.ph_count();
        let mut max_end_vpn = VirtPageNum(0);
        for i in 0..ph_count {
            let ph = elf.program_header(i).unwrap();
            if ph.get_type().unwrap() == program::Type::Load {
                let start_va = VirtAddr(ph.virtual_addr() as usize);
                let end_va = VirtAddr(start_va.0 + ph.mem_size() as usize);
                let mut map_perm = MapPermission::U;
                let ph_flags = ph.flags();
                if ph_flags.is_read() {
                    map_perm |= MapPermission::R;
                }
                if ph_flags.is_write() {
                    map_perm |= MapPermission::W;
                }
                if ph_flags.is_execute() {
                    map_perm |= MapPermission::X;
                }
                let map_area = MapArea::new(
                    start_va,
                    end_va,
                    MapType::Framed {
                        data_frames: Default::default(),
                    },
                    map_perm,
                );
                max_end_vpn = map_area.vpn_range.end;
                memory_set.push(
                    map_area,
                    Some(&elf.input[ph.offset() as usize..(ph.offset() + ph.file_size()) as usize]),
                )
            }
        }
        let max_end_va = max_end_vpn.page_start();
        let mut user_stack_bottom = max_end_va.0;
        // 作为 Guard Page
        user_stack_bottom += PAGE_SIZE;
        let user_stack_top = user_stack_bottom + USER_STACK_SIZE;
        memory_set.push(
            MapArea::new(
                VirtAddr(user_stack_bottom),
                VirtAddr(user_stack_top),
                MapType::Framed {
                    data_frames: Default::default(),
                },
                MapPermission::R | MapPermission::W | MapPermission::U,
            ),
            None,
        );
        // Trap Context
        memory_set.push(
            MapArea::new(
                VirtAddr(TRAP_CONTEXT),
                VirtAddr(TRAMPOLINE),
                MapType::Framed {
                    data_frames: Default::default(),
                },
                MapPermission::R | MapPermission::W,
            ),
            None,
        );
        (
            memory_set,
            user_stack_top,
            elf_header.pt2.entry_point() as usize,
        )
    }
    /// 映射跳板，也就是进入和退出异常处理的地方。
    ///
    /// 无论对于内核还是应用，跳板都位于虚拟地址空间的最高一页。
    ///
    /// 而它们都被实际映射到同一个物理帧，即 linker 指定的 strampoline
    fn map_trampoline(&mut self) {
        log::trace!("mapping trampoline");
        self.page_table.map(
            VirtAddr(TRAMPOLINE).floor(),
            PhysAddr(strampoline as usize).floor(),
            PTEFlags::R | PTEFlags::X,
        )
    }
    pub fn satp(&self) -> usize {
        self.page_table.satp()
    }
    pub fn translate(&self, vpn: VirtPageNum) -> Option<PageTableEntry> {
        self.page_table.translate(vpn)
    }
}

#[allow(unused)]
pub fn remap_test() {
    let mut kernel_space = KERNEL_SPACE.exclusive_access();
    let mid_text: VirtAddr = VirtAddr((stext as usize + etext as usize) / 2);
    let mid_rodata: VirtAddr = VirtAddr((srodata as usize + erodata as usize) / 2);
    let mid_data: VirtAddr = VirtAddr((sdata as usize + edata as usize) / 2);
    assert!(!kernel_space
        .page_table
        .translate(mid_text.floor())
        .unwrap()
        .writable());
    assert!(!kernel_space
        .page_table
        .translate(mid_rodata.floor())
        .unwrap()
        .writable());
    assert!(!kernel_space
        .page_table
        .translate(mid_data.floor())
        .unwrap()
        .executable());
    log::info!("remap_test passed!");
}

/// Get the token of the kernel memory space
pub fn kernel_stap() -> usize {
    KERNEL_SPACE.exclusive_access().satp()
}
