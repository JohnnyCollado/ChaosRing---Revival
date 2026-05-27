/*
 * This Source Code Form is subject to the terms of the Mozilla Public
 * License, v. 2.0. If a copy of the MPL was not distributed with this
 * file, You can obtain one at https://mozilla.org/MPL/2.0/.
 */
//! `NSFileManager` etc.

use super::{ns_array, ns_string, NSUInteger};
use crate::dyld::{export_c_func, ConstantExports, FunctionExports, HostConstant};
use crate::frameworks::foundation::ns_error::{NSCocoaErrorDomain, NSFileReadNoSuchFileError};
use crate::frameworks::foundation::ns_string::get_static_str;
use crate::fs::{FsError, GuestPath, GuestPathBuf};
use crate::mem::{ConstPtr, MutPtr, Ptr};
use crate::objc::{
    autorelease, id, msg, msg_class, nil, objc_classes, release, ClassExports, HostObject,
};
use crate::Environment;

type NSSearchPathDirectory = NSUInteger;
const NSApplicationDirectory: NSSearchPathDirectory = 1;
const NSLibraryDirectory: NSSearchPathDirectory = 5;
const NSDocumentDirectory: NSSearchPathDirectory = 9;

type NSSearchPathDomainMask = NSUInteger;
const NSUserDomainMask: NSSearchPathDomainMask = 1;

pub const NSFileModificationDate: &str = "NSFileModificationDate";
pub const NSFileSize: &str = "NSFileSize";
const NSFileSystemFreeSize: &str = "NSFileSystemFreeSize";
pub const NSFileType: &str = "NSFileType";
pub const NSFileTypeDirectory: &str = "NSFileTypeDirectory";
pub const NSFileTypeRegular: &str = "NSFileTypeRegular";
pub const NSFileTypeSymbolicLink: &str = "NSFileTypeSymbolicLink";
pub const NSFileTypeSocket: &str = "NSFileTypeSocket";
pub const NSFileTypeUnknown: &str = "NSFileTypeUnknown";
pub const NSFileCreationDate: &str = "NSFileCreationDate";
pub const NSFileSystemFileNumber: &str = "NSFileSystemFileNumber";
pub const NSFileSystemNumber: &str = "NSFileSystemNumber";
pub const NSFilePosixPermissions: &str = "NSFilePosixPermissions";
pub const NSFileOwnerAccountID: &str = "NSFileOwnerAccountID";
pub const NSFileGroupOwnerAccountID: &str = "NSFileGroupOwnerAccountID";
pub const NSFileReferenceCount: &str = "NSFileReferenceCount";
pub const NSFileExtensionHidden: &str = "NSFileExtensionHidden";
pub const NSFileImmutable: &str = "NSFileImmutable";
pub const NSFileAppendOnly: &str = "NSFileAppendOnly";
pub const NSFileBusy: &str = "NSFileBusy";
pub const NSFileHFSCreatorCode: &str = "NSFileHFSCreatorCode";
pub const NSFileHFSTypeCode: &str = "NSFileHFSTypeCode";

pub const CONSTANTS: ConstantExports = &[
    (
        "_NSFileModificationDate",
        HostConstant::NSString(NSFileModificationDate),
    ),
    ("_NSFileSize", HostConstant::NSString(NSFileSize)),
    (
        "_NSFileSystemFreeSize",
        HostConstant::NSString(NSFileSystemFreeSize),
    ),
    ("_NSFileType", HostConstant::NSString(NSFileType)),
    (
        "_NSFileTypeDirectory",
        HostConstant::NSString(NSFileTypeDirectory),
    ),
    (
        "_NSFileTypeRegular",
        HostConstant::NSString(NSFileTypeRegular),
    ),
    (
        "_NSFileTypeSymbolicLink",
        HostConstant::NSString(NSFileTypeSymbolicLink),
    ),
    (
        "_NSFileTypeSocket",
        HostConstant::NSString(NSFileTypeSocket),
    ),
    (
        "_NSFileTypeUnknown",
        HostConstant::NSString(NSFileTypeUnknown),
    ),
    (
        "_NSFileCreationDate",
        HostConstant::NSString(NSFileCreationDate),
    ),
    (
        "_NSFileSystemFileNumber",
        HostConstant::NSString(NSFileSystemFileNumber),
    ),
    (
        "_NSFileSystemNumber",
        HostConstant::NSString(NSFileSystemNumber),
    ),
    (
        "_NSFilePosixPermissions",
        HostConstant::NSString(NSFilePosixPermissions),
    ),
    (
        "_NSFileOwnerAccountID",
        HostConstant::NSString(NSFileOwnerAccountID),
    ),
    (
        "_NSFileGroupOwnerAccountID",
        HostConstant::NSString(NSFileGroupOwnerAccountID),
    ),
    (
        "_NSFileReferenceCount",
        HostConstant::NSString(NSFileReferenceCount),
    ),
    (
        "_NSFileExtensionHidden",
        HostConstant::NSString(NSFileExtensionHidden),
    ),
    (
        "_NSFileImmutable",
        HostConstant::NSString(NSFileImmutable),
    ),
    (
        "_NSFileAppendOnly",
        HostConstant::NSString(NSFileAppendOnly),
    ),
    ("_NSFileBusy", HostConstant::NSString(NSFileBusy)),
    (
        "_NSFileHFSCreatorCode",
        HostConstant::NSString(NSFileHFSCreatorCode),
    ),
    (
        "_NSFileHFSTypeCode",
        HostConstant::NSString(NSFileHFSTypeCode),
    ),
];

fn NSSearchPathForDirectoriesInDomains(
    env: &mut Environment,
    directory: NSSearchPathDirectory,
    domain_mask: NSSearchPathDomainMask,
    expand_tilde: bool,
) -> id {
    // TODO: other cases not implemented
    assert!(domain_mask == NSUserDomainMask);
    assert!(expand_tilde);

    let dir = match directory {
        NSApplicationDirectory => {
            // This might not actually be correct. I haven't bothered to
            // test it because I can't think of a good reason an iPhone OS app
            // would have to request this;
            // Wolfenstein 3D requests it but never uses it.
            GuestPath::new(crate::fs::APPLICATIONS).to_owned()
        }
        NSDocumentDirectory => env.fs.home_directory().join("Documents"),
        NSLibraryDirectory => env.fs.home_directory().join("Library"),
        _ => todo!("NSSearchPathDirectory {}", directory),
    };
    let dir = ns_string::from_rust_string(env, String::from(dir));
    let dir_list = ns_array::from_vec(env, vec![dir]);
    autorelease(env, dir_list)
}

fn NSHomeDirectory(env: &mut Environment) -> id {
    let dir = env.fs.home_directory();
    let dir = ns_string::from_rust_string(env, String::from(dir.as_str()));
    autorelease(env, dir)
}

/// Check [crate::fs::Fs::new] for more info for
/// how temporary folder is setup on startup
fn NSTemporaryDirectory(env: &mut Environment) -> id {
    let dir = env.fs.home_directory().join("tmp");
    let dir = ns_string::from_rust_string(env, String::from(dir.as_str()));
    autorelease(env, dir)
}

pub const FUNCTIONS: FunctionExports = &[
    export_c_func!(NSHomeDirectory()),
    export_c_func!(NSTemporaryDirectory()),
    export_c_func!(NSSearchPathForDirectoriesInDomains(_, _, _)),
];

#[derive(Default)]
pub struct State {
    default_manager: Option<id>,
}

struct NSDirectoryEnumeratorHostObject {
    iterator: std::vec::IntoIter<GuestPathBuf>,
}
impl HostObject for NSDirectoryEnumeratorHostObject {}

pub const CLASSES: ClassExports = objc_classes! {

(env, this, _cmd);

@implementation NSFileManager: NSObject

+ (id)defaultManager {
    if let Some(existing) = env.framework_state.foundation.ns_file_manager.default_manager {
        existing
    } else {
        let new: id = msg![env; this new];
        env.framework_state.foundation.ns_file_manager.default_manager = Some(new);
        new
    }
}

- (id)currentDirectoryPath {
    ns_string::from_rust_string(env, env.fs.working_directory().as_str().to_string())
}

- (bool)changeCurrentDirectoryPath:(id)path {
    let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
    let path = GuestPath::new(&path);
    match env.fs.change_working_directory(path) {
        Ok(_) => true,
        Err(()) => false
    }
}

- (bool)fileExistsAtPath:(id)path { // NSString*
    let res_exists = if path == nil {
        false
    } else {
        let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
        // fileExistsAtPath: will return true for directories
        // hence Fs::exists() rather than Fs::is_file() is appropriate.
        env.fs.exists(GuestPath::new(&path))
    };
    log_dbg!("[(NSFileManager*) {:?} fileExistsAtPath:{:?}] => {}", this, path, res_exists);
    res_exists
}

- (bool)fileExistsAtPath:(id)path // NSString*
             isDirectory:(MutPtr<bool>)is_dir {
    let (res_exists, res_is_dir) = if path == nil {
        (false, false)
    } else {
        // TODO: mutualize with fileExistsAtPath:
        let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
        let guest_path = GuestPath::new(&path);
        (env.fs.exists(guest_path), !env.fs.is_file(guest_path))
    };

    if !is_dir.is_null() {
        env.mem.write(is_dir, res_is_dir);
    }

    log_dbg!("[(NSFileManager*) {:?} fileExistsAtPath:{:?} isDirectory:{:?}] => {}", this, path, res_is_dir, res_exists);
    res_exists
}

- (bool)createFileAtPath:(id)path // NSString*
                contents:(id)data // NSData*
              attributes:(id)attributes { // NSDictionary*
    assert!(attributes == nil); // TODO
    if data == nil {
        let empty: id = msg_class![env; NSData new];
        let res: bool = msg![env; empty writeToFile:path atomically:false];
        release(env, empty);
        res
    } else {
        msg![env; data writeToFile:path atomically:false]
    }
}

- (bool)removeItemAtPath:(id)path // NSString*
                   error:(MutPtr<id>)out_error { // NSError**
    // TODO: call delegate
    let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
    match env.fs.remove(GuestPath::new(&path)) {
        Ok(()) => true,
        Err(err) => {
            if !out_error.is_null() {
                match err {
                    FsError::DoesNotExist => {
                        let domain = get_static_str(env, NSCocoaErrorDomain);
                        let error = msg_class![env; NSError alloc];
                        let error = msg![env; error initWithDomain:domain code:NSFileReadNoSuchFileError userInfo:nil];
                        autorelease(env, error);
                        env.mem.write(out_error, error);
                    }
                    _ => unimplemented!()
                }
            }
            false
        }
    }
}

- (bool)moveItemAtPath:(id)path // NSString*
                toPath:(id)toPath // NSString*
                 error:(MutPtr<id>)error { // NSError**
    // TODO: call delegate
    let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
    let toPath = ns_string::to_rust_string(env, toPath); // TODO: avoid copy
    match env.fs.rename(GuestPath::new(&path), GuestPath::new(&toPath)) {
        Ok(()) => true,
        Err(()) => {
            if !error.is_null() {
               todo!(); // TODO: create an NSError if requested
            }
            false
        }
    }
}

- (bool)createDirectoryAtPath:(id)path // NSString *
                   attributes:(id)attributes { // NSDictionary*
    let error: MutPtr<id> = Ptr::null();
    msg![env; this createDirectoryAtPath:path
             withIntermediateDirectories:false
                              attributes:attributes
                                   error:error]
}

- (bool)createDirectoryAtPath:(id)path // NSString *
  withIntermediateDirectories:(bool)with_intermediates
                   attributes:(id)attributes // NSDictionary*
                        error:(MutPtr<id>)error { // NSError**
    assert_eq!(attributes, nil); // TODO

    let path_str = ns_string::to_rust_string(env, path); // TODO: avoid copy
    let res = if with_intermediates {
        env.fs.create_dir_all(GuestPath::new(&path_str))
    } else {
        env.fs.create_dir(GuestPath::new(&path_str))
    };
    match res {
        Ok(()) => {
            log_dbg!("createDirectoryAtPath {} => true", path_str);
            true
        }
        Err(err) => {
            assert!(error.is_null()); // TODO
            log!(
                "Warning: createDirectoryAtPath {} failed with {:?}, returning false",
                path_str,
                err,
            );
            false
        }
    }
}

- (id)enumeratorAtPath:(id)path { // NSString*
    let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
    let Ok(paths) = env.fs.enumerate_recursive(GuestPath::new(&path)) else {
        return nil;
    };
    let host_object = Box::new(NSDirectoryEnumeratorHostObject {
        iterator: paths.into_iter(),
    });
    let class = env.objc.get_known_class("NSDirectoryEnumerator", &mut env.mem);
    let enumerator = env.objc.alloc_object(class, host_object, &mut env.mem);
    autorelease(env, enumerator)
}

- (id)directoryContentsAtPath:(id)path /* NSString* */ { // NSArray*
    let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
    let Ok(paths) = env.fs.enumerate(GuestPath::new(&path)) else {
        return nil;
    };
    let paths: Vec<GuestPathBuf> = paths
        .map(|path| GuestPathBuf::from(GuestPath::new(path)))
        .collect();
    log_dbg!("directoryContentsAtPath {}: {:?}", path, paths);
    let path_strings = paths
        .iter()
        .map(|name| ns_string::from_rust_string(env, name.as_str().to_string()))
        .collect();
    let res = ns_array::from_vec(env, path_strings);
    autorelease(env, res)
}

- (id)contentsOfDirectoryAtPath:(id)path /* NSString* */
                          error:(MutPtr<id>)error { // NSError**
    let contents: id = msg![env; this directoryContentsAtPath:path];
    if contents == nil && !error.is_null() {
        todo!(); // TODO: create an NSError if requested
    }
    contents
}

- (bool)isReadableFileAtPath:(id)path { // NSString*
    let (_, readable, _, _) = {
        let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
        env.fs.access(GuestPath::new(&path))
    };
    readable
}

- (bool)isWritableFileAtPath:(id)path { // NSString*
    let (_, _, writable, _) = {
        let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
        env.fs.access(GuestPath::new(&path))
    };
    writable
}

- (bool)isDeletableFileAtPath:(id)path { // NSString*
    let is_file = {
        let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
        env.fs.is_file(GuestPath::new(&path))
    };

    if is_file {
        return msg![env; this isWritableFileAtPath:path];
    }

    let directory_enumerator: id = msg![env; this enumeratorAtPath:path];

    let mut is_deletable = true;
    loop {
        let path: id = msg![env; directory_enumerator nextObject];
        if path == nil {
            break;
        }
        let is_path_deletable: bool = msg![env; this isDeletableFileAtPath:path];
        is_deletable &= is_path_deletable;
        if !is_deletable {
            break;
        }
    }
    is_deletable
}

- (id)contentsAtPath:(id)path { // NSString *
    // TODO: return nil if path is directory
    // TODO: handle non-absolute paths?
    assert!(msg![env; path isAbsolutePath]);
    msg_class![env; NSData dataWithContentsOfFile:path]
}

- (bool)copyItemAtPath:(id)src // NSString*
                toPath:(id)dst // NSString*
                 error:(MutPtr<id>)error { // NSError**
    let src = ns_string::to_rust_string(env, src);
    let dst = ns_string::to_rust_string(env, dst);
    let data = match env.fs.read(GuestPath::new(src.as_ref())) {
        Ok(d) => d,
        Err(_) => {
            assert!(error.is_null()); // TODO
            return false;
        }
    };
    if env.fs.write(GuestPath::new(dst.as_ref()), &data).is_err() {
        assert!(error.is_null()); // TODO
        return false;
    }
    true
}

- (ConstPtr<u8>)fileSystemRepresentationWithPath:(id)path { // NSString*
    let length: NSUInteger = msg![env; path length];
    assert!(length > 0);
    // TODO: throw an exception if conversion fails
    msg![env; path UTF8String]
}

- (id)fileAttributesAtPath:(id)path // NSString *
              traverseLink:(bool)traverse {
    // TODO: other attributes
    log_once!("Warning: NSFileManager fileAttributesAtPath:traverseLink: returns only NSFileType, NSFileModificationDate and NSFileSize attributes!");

    let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
    // TODO: traverse link
    log_dbg!("[(NSFileManager *){:?} fileAttributesAtPath:{} traverse:{}]", this, path, traverse);
    let guest_path = GuestPath::new(&path);

    file_attributes_common(env, guest_path)
}

- (id)attributesOfItemAtPath:(id)path // NSString *
                       error:(MutPtr<id>)error { // NSError **
    assert!(error.is_null()); // TODO

    let path = ns_string::to_rust_string(env, path); // TODO: avoid copy
    // TODO: traverse symlink (touchHLE's filesystem doesn't model them yet).
    log_dbg!("[(NSFileManager *){:?} attributesOfItemAtPath:{} error:{:?}]", this, path, error);
    let guest_path = GuestPath::new(&path);

    file_attributes_common(env, guest_path)
}

- (id)attributesOfFileSystemForPath:(id)_path
                              error:(MutPtr<id>)error {
    // TODO: other attributes
    log_once!("Warning: NSFileManager attributesOfFileSystemForPath:error: returns only NSFileSystemFreeSize attribute!");

    assert!(error.is_null()); // TODO

    let dict = msg_class![env; NSMutableDictionary new];

    // Reporting 1 Gb of free space should be enough
    // TODO: unify with `statfs`
    // TODO: account for path
    let size: u64 = 1024 * 1024 * 1024;
    let size_num: id = msg_class![env; NSNumber numberWithUnsignedLongLong:size];

    let fs_free_size_key = get_static_str(env, NSFileSystemFreeSize);
    () = msg![env; dict setObject:size_num forKey:fs_free_size_key];

    let dict_imm = msg![env; dict copy];
    release(env, dict);
    autorelease(env, dict_imm)
}

@end

@implementation NSDirectoryEnumerator: NSEnumerator

- (id)nextObject {
    let host_obj = env.objc.borrow_mut::<NSDirectoryEnumeratorHostObject>(this);
    host_obj.iterator.next().map_or(nil, |s| ns_string::from_rust_string(env, String::from(s)))
}

@end

};

/// Helper function for `fileAttributesAtPath:traverseLink:` and
/// `attributesOfItemAtPath:error:`.
///
/// Returns a dictionary populated with **every** common iOS attribute key,
/// using sensible defaults where we don't track the real value. Guest code
/// commonly does `[[dict objectForKey:NSFilePosixPermissions] intValue]`,
/// and getting nil back here can propagate as 0, undefined behaviour, or
/// the C++-exception-into-Rust cascade we hit while loading saves.
fn file_attributes_common(env: &mut Environment, guest_path: &GuestPath) -> id {
    if !env.fs.exists(guest_path) {
        log_dbg!(
            "file_attributes_common() called with file that does not exist: {:?}, Returning nil",
            guest_path
        );
        return nil;
    }

    let is_dir = env.fs.is_dir(guest_path);
    let unix_timestamp: f64 = env.fs.modified(guest_path).unwrap() as f64;
    let size: u64 = if is_dir {
        0
    } else {
        env.fs.size(guest_path).unwrap_or(0)
    };

    // Build a stable, non-zero pseudo-inode from the path. The exact value
    // doesn't matter; what matters is that it's distinct between distinct
    // paths and stable across queries for the same path within a session.
    let inode_pseudo: u64 = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        guest_path.as_str().hash(&mut h);
        // OR a non-zero bit so we never produce 0 (which guest code might
        // treat as "no inode").
        h.finish() | 1
    };

    let unix_ref_date: id = msg_class![env; NSDate dateWithTimeIntervalSince1970:0f64];
    let unix_date: id =
        msg_class![env; NSDate dateWithTimeInterval:unix_timestamp sinceDate:unix_ref_date];

    let size_num: id = msg_class![env; NSNumber numberWithUnsignedLongLong:size];
    let inode_num: id = msg_class![env; NSNumber numberWithUnsignedLongLong:inode_pseudo];
    let fs_number_num: id = {
        let one: u32 = 1;
        msg_class![env; NSNumber numberWithUnsignedInt:one]
    };
    let perms_num: id = {
        let perms: u32 = if is_dir { 0o755 } else { 0o644 };
        msg_class![env; NSNumber numberWithUnsignedInt:perms]
    };
    let uid_num: id = {
        let uid: u32 = 501;
        msg_class![env; NSNumber numberWithUnsignedInt:uid]
    };
    let gid_num: id = {
        let gid: u32 = 501;
        msg_class![env; NSNumber numberWithUnsignedInt:gid]
    };
    let refcount_num: id = {
        let n: u32 = 1;
        msg_class![env; NSNumber numberWithUnsignedInt:n]
    };
    let zero_u32_num: id = {
        let n: u32 = 0;
        msg_class![env; NSNumber numberWithUnsignedInt:n]
    };
    let false_num: id = msg_class![env; NSNumber numberWithBool:false];

    let dict = msg_class![env; NSMutableDictionary new];

    // Type
    let file_type_key = get_static_str(env, NSFileType);
    let file_type_value = if is_dir {
        get_static_str(env, NSFileTypeDirectory)
    } else if env.fs.is_file(guest_path) {
        get_static_str(env, NSFileTypeRegular)
    } else {
        get_static_str(env, NSFileTypeUnknown)
    };
    () = msg![env; dict setObject:file_type_value forKey:file_type_key];

    // Size + dates
    let size_key = get_static_str(env, NSFileSize);
    () = msg![env; dict setObject:size_num forKey:size_key];
    let modif_date_key = get_static_str(env, NSFileModificationDate);
    () = msg![env; dict setObject:unix_date forKey:modif_date_key];
    // We don't track creation time separately; use modification time.
    let creation_date_key = get_static_str(env, NSFileCreationDate);
    () = msg![env; dict setObject:unix_date forKey:creation_date_key];

    // Identity / linkage
    let inode_key = get_static_str(env, NSFileSystemFileNumber);
    () = msg![env; dict setObject:inode_num forKey:inode_key];
    let fs_num_key = get_static_str(env, NSFileSystemNumber);
    () = msg![env; dict setObject:fs_number_num forKey:fs_num_key];
    let refcount_key = get_static_str(env, NSFileReferenceCount);
    () = msg![env; dict setObject:refcount_num forKey:refcount_key];

    // POSIX-ish
    let perms_key = get_static_str(env, NSFilePosixPermissions);
    () = msg![env; dict setObject:perms_num forKey:perms_key];
    let uid_key = get_static_str(env, NSFileOwnerAccountID);
    () = msg![env; dict setObject:uid_num forKey:uid_key];
    let gid_key = get_static_str(env, NSFileGroupOwnerAccountID);
    () = msg![env; dict setObject:gid_num forKey:gid_key];

    // Flags (all false / zero by default)
    let ext_hidden_key = get_static_str(env, NSFileExtensionHidden);
    () = msg![env; dict setObject:false_num forKey:ext_hidden_key];
    let immutable_key = get_static_str(env, NSFileImmutable);
    () = msg![env; dict setObject:false_num forKey:immutable_key];
    let appendonly_key = get_static_str(env, NSFileAppendOnly);
    () = msg![env; dict setObject:false_num forKey:appendonly_key];
    let busy_key = get_static_str(env, NSFileBusy);
    () = msg![env; dict setObject:false_num forKey:busy_key];

    // HFS legacy
    let hfs_creator_key = get_static_str(env, NSFileHFSCreatorCode);
    () = msg![env; dict setObject:zero_u32_num forKey:hfs_creator_key];
    let hfs_type_key = get_static_str(env, NSFileHFSTypeCode);
    () = msg![env; dict setObject:zero_u32_num forKey:hfs_type_key];

    let dict_imm: id = msg![env; dict copy];
    release(env, dict);
    autorelease(env, dict_imm)
}
