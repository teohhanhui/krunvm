// Copyright 2021 Red Hat, Inc.
// SPDX-License-Identifier: Apache-2.0

use std::ffi::CString;
use std::fs::File;
use std::os::unix::io::AsRawFd;
use std::path::Path;

use super::bindings;
use super::utils::{mount_container, umount_container};
use crate::{ArgMatches, KrunvmConfig, VmConfig};

unsafe fn exec_vm(vmcfg: &VmConfig, rootfs: &str, cmd: &str, args: Vec<CString>) {
    //bindings::krun_set_log_level(9);

    let ctx = bindings::krun_create_ctx() as u32;

    let ret = bindings::krun_set_vm_config(ctx, vmcfg.cpus as u8, vmcfg.mem);
    if ret < 0 {
        println!("Error setting VM config");
        std::process::exit(-1);
    }

    let c_rootfs = CString::new(rootfs).unwrap();
    let ret = bindings::krun_set_root(ctx, c_rootfs.as_ptr());
    if ret < 0 {
        println!("Error setting VM rootfs");
        std::process::exit(-1);
    }

    let mut volumes = Vec::new();
    for (host_path, guest_path) in vmcfg.mapped_volumes.iter() {
        let full_guest = format!("{}{}", &rootfs, guest_path);
        let full_guest_path = Path::new(&full_guest);
        if !full_guest_path.exists() {
            std::fs::create_dir(full_guest_path)
                .expect("Couldn't create guest_path for mapped volume");
        }
        let map = format!("{}:{}", host_path, guest_path);
        volumes.push(CString::new(map).unwrap());
    }
    let mut vols: Vec<*const i8> = Vec::new();
    for vol in volumes.iter() {
        vols.push(vol.as_ptr());
    }
    vols.push(std::ptr::null());
    let ret = bindings::krun_set_mapped_volumes(ctx, vols.as_ptr());
    if ret < 0 {
        println!("Error setting VM mapped volumes");
        std::process::exit(-1);
    }

    let mut ports = Vec::new();
    for (host_port, guest_port) in vmcfg.mapped_ports.iter() {
        let map = format!("{}:{}", host_port, guest_port);
        ports.push(CString::new(map).unwrap());
    }
    let mut ps: Vec<*const i8> = Vec::new();
    for port in ports.iter() {
        ps.push(port.as_ptr());
    }
    ps.push(std::ptr::null());
    let ret = bindings::krun_set_port_map(ctx, ps.as_ptr());
    if ret < 0 {
        println!("Error setting VM port map");
        std::process::exit(-1);
    }

    let c_workdir = CString::new(vmcfg.workdir.clone()).unwrap();
    let ret = bindings::krun_set_workdir(ctx, c_workdir.as_ptr());
    if ret < 0 {
        println!("Error setting VM workdir");
        std::process::exit(-1);
    }

    let mut argv: Vec<*const i8> = Vec::new();
    for a in args.iter() {
        argv.push(a.as_ptr());
    }
    argv.push(std::ptr::null());

    let hostname = CString::new(format!("HOSTNAME={}", vmcfg.name)).unwrap();
    let home = CString::new("HOME=/root").unwrap();
    let path = CString::new("PATH=/bin:/sbin:/usr/bin:/usr/sbin:/usr/local/bin").unwrap();
    let env: [*const i8; 4] = [
        hostname.as_ptr(),
        home.as_ptr(),
        path.as_ptr(),
        std::ptr::null(),
    ];

    let c_cmd = CString::new(cmd).unwrap();
    let ret = bindings::krun_set_exec(ctx, c_cmd.as_ptr(), argv.as_ptr(), env.as_ptr());
    if ret < 0 {
        println!("Error setting VM config");
        std::process::exit(-1);
    }

    let ret = bindings::krun_start_enter(ctx);
    if ret < 0 {
        println!("Error starting VM");
        std::process::exit(-1);
    }
}

fn set_rlimits() {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };

    let ret = unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &mut limit) };
    if ret < 0 {
        panic!("Couldn't get RLIMIT_NOFILE value");
    }

    limit.rlim_cur = limit.rlim_max;
    let ret = unsafe { libc::setrlimit(libc::RLIMIT_NOFILE, &limit) };
    if ret < 0 {
        panic!("Couldn't set RLIMIT_NOFILE value");
    }
}

fn set_lock(rootfs: &str) -> File {
    let lock_path = format!("{}/.krunvm.lock", rootfs);
    let file = File::create(lock_path).expect("Couldn't create lock file");

    let ret = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if ret < 0 {
        println!("Couldn't acquire lock file. Is another instance of this VM already running?");
        std::process::exit(-1);
    }

    file
}

pub fn start(cfg: &KrunvmConfig, matches: &ArgMatches) {
    let cmd = matches.value_of("COMMAND").unwrap();
    let name = matches.value_of("NAME").unwrap();

    let vmcfg = match cfg.vmconfig_map.get(name) {
        None => {
            println!("No VM found with name {}", name);
            std::process::exit(-1);
        }
        Some(vmcfg) => vmcfg,
    };

    umount_container(&cfg, vmcfg).expect("Error unmounting container");
    let rootfs = mount_container(&cfg, vmcfg).expect("Error mounting container");

    let args: Vec<CString> = match matches.values_of("ARGS") {
        Some(a) => a.map(|val| CString::new(val).unwrap()).collect(),
        None => Vec::new(),
    };

    set_rlimits();

    let _file = set_lock(&rootfs);

    unsafe { exec_vm(vmcfg, &rootfs, cmd, args) };

    umount_container(&cfg, vmcfg).expect("Error unmounting container");
}