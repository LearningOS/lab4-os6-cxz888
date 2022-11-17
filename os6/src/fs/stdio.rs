use crate::{mm::page_table::UserBuffer, sbi, task};

use super::{File, Stat, StatMode};

pub struct Stdin;
pub struct Stdout;

impl File for Stdin {
    fn readable(&self) -> bool {
        true
    }
    fn writable(&self) -> bool {
        true
    }
    fn read(&self, mut buf: UserBuffer) -> usize {
        assert_eq!(buf.len(), 1);
        let c = loop {
            let c = sbi::console_getchar() as u8;
            if c == 0 {
                task::suspend_current_and_run_next();
            } else {
                break c;
            }
        };
        unsafe { buf.buffers[0].as_mut_ptr().write_volatile(c) }
        1
    }
    fn write(&self, _buf: UserBuffer) -> usize {
        panic!("Cannot write to stdin");
    }
    fn stat(&self) -> Stat {
        Stat {
            dev: 0,
            ino: 0,
            mode: StatMode::NULL,
            nlink: 1,
            pad: [0; 7],
        }
    }
}

impl File for Stdout {
    fn readable(&self) -> bool {
        false
    }
    fn writable(&self) -> bool {
        true
    }
    fn read(&self, mut _buf: UserBuffer) -> usize {
        panic!("Cannot read from stdout");
    }
    fn write(&self, buf: UserBuffer) -> usize {
        for buffer in &buf.buffers {
            print!("{}", core::str::from_utf8(*buffer).unwrap());
        }
        buf.len()
    }
    fn stat(&self) -> Stat {
        super::Stat {
            dev: 0,
            ino: 0,
            mode: StatMode::NULL,
            nlink: 1,
            pad: [0; 7],
        }
    }
}
