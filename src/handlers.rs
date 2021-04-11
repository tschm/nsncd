/*
 * Copyright 2020 Two Sigma Open Source, LLC
 *
 * Licensed under the Apache License, Version 2.0 (the "License");
 * you may not use this file except in compliance with the License.
 * You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

use std::convert::TryInto;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;

use anyhow::{Context, Result};
use atoi::atoi;
use nix::unistd::{Gid, Group, Uid, User};
use slog::{debug, Logger};

use super::protocol;
use super::protocol::RequestType;

/// Handle a request by performing the appropriate lookup and sending the
/// serialized response back to the client.
///
/// # Arguments
///
/// * `log` - A `slog` Logger.
/// * `request` - The request to handle.
pub fn handle_request(log: &Logger, request: &protocol::Request) -> Result<Vec<u8>> {
    debug!(log, "handling request"; "request" => ?request);
    match request.ty {
        RequestType::GETPWBYUID => {
            let key = CStr::from_bytes_with_nul(request.key)?;
            let uid = atoi::<u32>(key.to_bytes()).context("invalid uid string")?;
            let user = User::from_uid(Uid::from_raw(uid))?;
            debug!(log, "got user"; "user" => ?user);
            serialize_user(user)
        }
        RequestType::GETPWBYNAME => {
            let key = CStr::from_bytes_with_nul(request.key)?;
            let user = User::from_name(key.to_str()?)?;
            debug!(log, "got user"; "user" => ?user);
            serialize_user(user)
        }
        RequestType::GETGRBYGID => {
            let key = CStr::from_bytes_with_nul(request.key)?;
            let gid = atoi::<u32>(key.to_bytes()).context("invalid gid string")?;
            let group = Group::from_gid(Gid::from_raw(gid))?;
            debug!(log, "got group"; "group" => ?group);
            serialize_group(group)
        }
        RequestType::GETGRBYNAME => {
            let key = CStr::from_bytes_with_nul(request.key)?;
            let group = Group::from_name(key.to_str()?)?;
            debug!(log, "got group"; "group" => ?group);
            serialize_group(group)
        }
        RequestType::GETHOSTBYADDR
        | RequestType::GETHOSTBYADDRv6
        | RequestType::GETHOSTBYNAME
        | RequestType::GETHOSTBYNAMEv6
        | RequestType::SHUTDOWN
        | RequestType::GETSTAT
        | RequestType::INVALIDATE
        | RequestType::GETFDPW
        | RequestType::GETFDGR
        | RequestType::GETFDHST
        | RequestType::GETAI
        | RequestType::GETSERVBYNAME
        | RequestType::GETSERVBYPORT
        | RequestType::GETFDSERV
        | RequestType::GETFDNETGR
        | RequestType::GETNETGRENT
        | RequestType::INNETGR
        | RequestType::LASTREQ
        | RequestType::INITGROUPS => Ok(vec![]),
    }
}

/// Send a user (passwd entry) back to the client, or a response indicating the
/// lookup found no such user.
fn serialize_user(user: Option<User>) -> Result<Vec<u8>> {
    let mut result = vec![];
    if let Some(data) = user {
        let name = CString::new(data.name)?;
        let name_bytes = name.to_bytes_with_nul();
        let passwd_bytes = data.passwd.to_bytes_with_nul();
        let gecos_bytes = data.gecos.to_bytes_with_nul();
        let dir = CString::new(data.dir.as_os_str().as_bytes())?;
        let dir_bytes = dir.to_bytes_with_nul();
        let shell = CString::new(data.shell.as_os_str().as_bytes())?;
        let shell_bytes = shell.to_bytes_with_nul();

        let header = protocol::PwResponseHeader {
            version: protocol::VERSION,
            found: 1,
            pw_name_len: name_bytes.len().try_into()?,
            pw_passwd_len: passwd_bytes.len().try_into()?,
            pw_uid: data.uid.as_raw(),
            pw_gid: data.gid.as_raw(),
            pw_gecos_len: gecos_bytes.len().try_into()?,
            pw_dir_len: dir_bytes.len().try_into()?,
            pw_shell_len: shell_bytes.len().try_into()?,
        };
        result.extend_from_slice(header.as_slice());
        result.extend_from_slice(name_bytes);
        result.extend_from_slice(passwd_bytes);
        result.extend_from_slice(gecos_bytes);
        result.extend_from_slice(dir_bytes);
        result.extend_from_slice(shell_bytes);
    } else {
        let header = protocol::PwResponseHeader::default();
        result.extend_from_slice(header.as_slice());
    }
    Ok(result)
}

/// Send a group (group entry) back to the client, or a response indicating the
/// lookup found no such group.
fn serialize_group(group: Option<Group>) -> Result<Vec<u8>> {
    let mut result = vec![];
    if let Some(data) = group {
        let name = CString::new(data.name)?;
        let name_bytes = name.to_bytes_with_nul();
        // The nix crate doesn't give us the password: https://github.com/nix-rust/nix/pull/1338
        let passwd = CString::new("x")?;
        let passwd_bytes = passwd.to_bytes_with_nul();
        let members: Vec<CString> = data
            .mem
            .iter()
            .map(|member| CString::new((*member).as_bytes()))
            .collect::<Result<Vec<CString>, _>>()?;
        let members_bytes: Vec<&[u8]> = members
            .iter()
            .map(|member| member.to_bytes_with_nul())
            .collect();

        let header = protocol::GrResponseHeader {
            version: protocol::VERSION,
            found: 1,
            gr_name_len: name_bytes.len().try_into()?,
            gr_passwd_len: passwd_bytes.len().try_into()?,
            gr_gid: data.gid.as_raw(),
            gr_mem_cnt: data.mem.len().try_into()?,
        };
        result.extend_from_slice(header.as_slice());
        for member_bytes in members_bytes.iter() {
            result.extend_from_slice(&i32::to_ne_bytes(member_bytes.len().try_into()?));
        }
        result.extend_from_slice(name_bytes);
        result.extend_from_slice(passwd_bytes);
        for member_bytes in members_bytes.iter() {
            result.extend_from_slice(member_bytes);
        }
    } else {
        let header = protocol::GrResponseHeader::default();
        result.extend_from_slice(header.as_slice());
    }
    Ok(result)
}

#[cfg(test)]
mod test {
    use super::*;
    use nix::libc::{c_int, gid_t, uid_t};

    fn test_logger() -> slog::Logger {
        Logger::root(slog::Discard, slog::o!())
    }

    #[test]
    fn test_handle_request_empty_key() {
        let request = protocol::Request {
            ty: protocol::RequestType::GETPWBYNAME,
            key: &[],
        };

        let result = handle_request(&test_logger(), &request);
        assert!(result.is_err(), "should error on empty input");
    }

    #[test]
    fn test_handle_request_nul_data() {
        let request = protocol::Request {
            ty: protocol::RequestType::GETPWBYNAME,
            key: &[0x7F, 0x0, 0x0, 0x01],
        };

        let result = handle_request(&test_logger(), &request);
        assert!(result.is_err(), "should error on garbage input");
    }

    #[test]
    fn test_handle_request_current_user() {
        let current_user = User::from_uid(nix::unistd::geteuid()).unwrap().unwrap();

        let request = protocol::Request {
            ty: protocol::RequestType::GETPWBYNAME,
            key: &CString::new(current_user.name.clone())
                .unwrap()
                .into_bytes_with_nul(),
        };

        let expected = serialize_user(Some(current_user))
            .expect("send_user should serialize current user data");
        let output =
            handle_request(&test_logger(), &request).expect("should handle request with no error");
        assert_eq!(expected, output);
    }

    #[test]
    fn test_serialize_user_notfound() {
        let mut expected = vec![];
        // pub version: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub found: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub pw_name_len: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub pw_passwd_len: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub pw_uid: uid_t,
        expected.extend_from_slice(&uid_t::from(0u32).to_ne_bytes());
        // pub pw_gid: gid_t,
        expected.extend_from_slice(&gid_t::from(0u32).to_ne_bytes());
        // pub pw_gecos_len: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub pw_dir_len: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub pw_shell_len: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        let output = serialize_user(None).unwrap();
        assert_eq!(expected, output);
    }

    #[test]
    fn test_serialize_user() {
        let user = User::from_name("root").unwrap().unwrap();
        let mut expected = vec![];
        // pub version: c_int,
        expected.extend_from_slice(&c_int::from(protocol::VERSION).to_ne_bytes());
        // pub found: c_int,
        expected.extend_from_slice(&c_int::from(1i32).to_ne_bytes());
        // pub pw_name_len: c_int,
        expected
            .extend_from_slice(&c_int::from(user.name.as_bytes().len() as i32 + 1).to_ne_bytes());
        // pub pw_passwd_len: c_int,
        expected.extend_from_slice(
            &c_int::from(user.passwd.as_bytes_with_nul().len() as i32).to_ne_bytes(),
        );
        // pub pw_uid: uid_t,
        expected.extend_from_slice(&user.uid.as_raw().to_ne_bytes());
        // pub pw_gid: gid_t,
        expected.extend_from_slice(&user.gid.as_raw().to_ne_bytes());
        // pub pw_gecos_len: c_int,
        expected.extend_from_slice(
            &c_int::from(user.gecos.as_bytes_with_nul().len() as i32).to_ne_bytes(),
        );
        // pub pw_dir_len: c_int,
        expected
            .extend_from_slice(&c_int::from(user.dir.as_os_str().len() as i32 + 1).to_ne_bytes());
        // pub pw_shell_len: c_int,
        expected
            .extend_from_slice(&c_int::from(user.shell.as_os_str().len() as i32 + 1).to_ne_bytes());
        expected.extend([user.name.as_bytes(), &[0u8]].concat());
        expected.extend(user.passwd.as_bytes_with_nul());
        expected.extend(user.gecos.as_bytes_with_nul());
        expected.extend([user.dir.as_os_str().as_bytes(), &[0u8]].concat());
        expected.extend([user.shell.as_os_str().as_bytes(), &[0u8]].concat());

        let output = serialize_user(Some(user)).unwrap();
        assert_eq!(expected, output);
    }

    #[test]
    fn test_serialize_group_notfound() {
        let mut expected = vec![];
        // pub version: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub found: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub gr_name_len: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub gr_passwd_len: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());
        // pub gr_gid: gid_t,
        expected.extend_from_slice(&gid_t::from(0u32).to_ne_bytes());
        // pub gr_mem_cnt: c_int,
        expected.extend_from_slice(&c_int::from(0i32).to_ne_bytes());

        let output = serialize_group(None).unwrap();
        assert_eq!(expected, output);
    }

    #[test]
    fn test_serialize_group() {
        let group = Group::from_name("root").unwrap().unwrap();
        let mut expected = vec![];
        // pub version: c_int,
        expected.extend_from_slice(&c_int::from(protocol::VERSION).to_ne_bytes());
        // pub found: c_int,
        expected.extend_from_slice(&c_int::from(1i32).to_ne_bytes());
        // pub gr_name_len: c_int,
        expected
            .extend_from_slice(&c_int::from(group.name.as_bytes().len() as i32 + 1).to_ne_bytes());
        // pub gr_passwd_len: c_int,
        expected.extend_from_slice(&c_int::from(2i32).to_ne_bytes());
        // pub gr_gid: gid_t,
        expected.extend_from_slice(&group.gid.as_raw().to_ne_bytes());
        // pub gr_mem_cnt: c_int,
        expected.extend_from_slice(&c_int::from(group.mem.len() as i32).to_ne_bytes());

        for mem in group.mem.iter() {
            expected.extend_from_slice(&c_int::from(mem.as_bytes().len() as i32 + 1).to_ne_bytes());
        }
        expected.extend([group.name.as_bytes(), &[0u8]].concat());
        expected.extend(["x".as_bytes(), &[0u8]].concat());
        for mem in group.mem.iter() {
            expected.extend([mem.as_bytes(), &[0u8]].concat());
        }

        let output = serialize_group(Some(group)).unwrap();
        assert_eq!(expected, output);
    }
}
