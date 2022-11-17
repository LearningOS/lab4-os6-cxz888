use crate::{
    fs::{
        self,
        inode::{OpenFlags, ROOT_INODE},
        Stat,
    },
    mm::page_table::{self, PageTable, UserBuffer},
    task::Processor,
};

pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    let task = Processor::current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if let Some(Some(file)) = inner.fd_table.get(fd) {
        let file = file.clone();
        assert!(file.writable());
        drop(inner);
        let satp = Processor::current_user_satp();
        file.write(UserBuffer::new(page_table::translated_byte_buffer(
            satp, buf, len,
        ))) as isize
    } else {
        -1
    }
}

/// 功能：从文件中读取一段内容到缓冲区。
///
/// 参数：fd 是待读取文件的文件描述符，切片 buffer 则给出缓冲区。
///
/// 返回值：如果出现了错误则返回 -1，否则返回实际读到的字节数。
///
/// syscall ID：63
pub fn sys_read(fd: usize, buf: *const u8, len: usize) -> isize {
    let task = Processor::current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if let Some(Some(file)) = inner.fd_table.get(fd) {
        let file = file.clone();
        assert!(file.readable());
        drop(inner);
        let satp = Processor::current_user_satp();
        file.read(UserBuffer::new(page_table::translated_byte_buffer(
            satp, buf, len,
        ))) as isize
    } else {
        -1
    }
}

/// 功能：打开一个常规文件，并返回可以访问它的文件描述符。
///
/// 参数：path 描述要打开的文件的文件名（简单起见，文件系统不需要支持目录，所有的文件都放在根目录 / 下）。
///
/// flag 表示打开文件的标志，具体含义如下：
///
/// - flags=0，表示只读，即 RDONLY
/// - flags\[0\]=1 即 flags=0x001，表示只写，即 WRONLY
/// - flags\[2\]=1 即 flags=0x002，表示可读可写，即 RDRW
/// - flags\[9\]=1 即 flags=0x200，表示创建文件，即 CREATE
/// - flags\[10\]=1 即 flags=0x400，表示打开文件时应该清空文件内容并将文件大小归零，即 TRUNC
///
/// 返回值：如果出现了错误则返回 -1，否则返回打开常规文件的文件描述符。可能的错误原因是：文件不存在。
///
/// syscall ID：56
pub fn sys_open(path: *const u8, flags: u32) -> isize {
    let flags = match OpenFlags::from_bits(flags) {
        Some(flags) => flags,
        None => return -1,
    };
    let user_satp = Processor::current_user_satp();
    let path = PageTable::translated_str(user_satp, path);
    let os_inode = match fs::open_file(&path, flags) {
        Some(os_inode) => os_inode,
        None => return -1,
    };
    let task = Processor::current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    let fd = inner.alloc_fd();
    inner.fd_table[fd] = Some(os_inode);
    fd as isize
}

/// 关闭文件。出错返回 -1，如传入的文件描述符并不对应一个打开的文件
///
/// syscall ID：57
pub fn sys_close(fd: usize) -> isize {
    let task = Processor::current_task().unwrap();
    let mut inner = task.inner_exclusive_access();
    match inner.fd_table.get_mut(fd) {
        Some(file) if file.is_some() => {
            file.take();
            0
        }
        _ => -1,
    }
}

/// 功能：创建一个文件的一个硬链接
///
/// 参数
/// - olddirfd, newdirfd: 仅为了兼容性考虑，本次实验中始终为 AT_FDCWD (-100)，可以忽略。
/// - flags: 仅为了兼容性考虑，本次实验中始终为 0，可以忽略。
/// - oldpath：原有文件路径
/// - newpath: 新的链接文件路径。
///
/// 返回值：如果出现了错误则返回 -1，否则返回 0。
///
/// syscall ID: 37
pub fn sys_linkat(
    _olddirfd: i32,
    oldpath: *const u8,
    _newdirfd: i32,
    newpath: *const u8,
    _flags: u32,
) -> isize {
    let satp = Processor::current_user_satp();
    let old_path = PageTable::translated_str(satp, oldpath);
    let new_path = PageTable::translated_str(satp, newpath);
    if old_path == new_path {
        return -1;
    }
    if ROOT_INODE.link(&old_path, &new_path) {
        0
    } else {
        -1
    }
}

/// 功能：取消一个文件路径到文件的链接
///
/// 参数：
/// - dirfd: 仅为了兼容性考虑，本次实验中始终为 AT_FDCWD (-100)，可以忽略
/// - path：文件路径
/// - flags: 仅为了兼容性考虑，本次实验中始终为 0，可以忽略
pub fn sys_unlinkat(_dirfd: i32, path: *const u8, _flags: u32) -> isize {
    let satp = Processor::current_user_satp();
    let path = PageTable::translated_str(satp, path);
    if ROOT_INODE.unlink(&path) {
        0
    } else {
        -1
    }
}

pub fn sys_fstat(fd: usize, st: *mut Stat) -> isize {
    let satp = Processor::current_user_satp();
    let st = PageTable::translated_mut(satp, st);
    st.dev = 0;
    let task = Processor::current_task().unwrap();
    let inner = task.inner_exclusive_access();
    if let Some(Some(inode)) = inner.fd_table.get(fd as usize) {
        *st = inode.stat();
        0
    } else {
        -1
    }
}
