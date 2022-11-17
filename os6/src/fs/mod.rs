pub mod inode;
pub mod stdio;

use crate::mm::page_table::UserBuffer;
use bitflags::bitflags;

bitflags! {
    /// StatMode 定义：
    pub struct StatMode: u32 {
        const NULL  = 0;
        /// directory
        const DIR   = 0o040000;
        /// ordinary regular file
        const FILE  = 0o100000;
    }
}

#[repr(C)]
pub struct Stat {
    /// 文件所在磁盘驱动器号，该实验中写死为 0 即可
    pub dev: u64,
    /// inode 文件所在 inode 编号
    pub ino: u64,
    /// 文件类型
    pub mode: StatMode,
    /// 硬链接数量，初始为 1
    pub nlink: u32,
    /// 无需考虑，为了兼容性设计
    pub pad: [u64; 7],
}

pub trait File: Send + Sync {
    fn readable(&self) -> bool;
    fn writable(&self) -> bool;
    fn read(&self, buf: UserBuffer) -> usize;
    fn write(&self, buf: UserBuffer) -> usize;
    fn stat(&self) -> Stat;
}

pub use inode::{list_apps, open_file};
