use std::{
    ffi::{c_char, c_int, c_uint, c_void, CStr, CString},
    path::Path,
    ptr, slice,
};

const MDB_RDONLY: c_uint = 0x20_000;
const MDB_NOLOCK: c_uint = 0x40_0000;
const MDB_NOTFOUND: c_int = -30_798;
const MDB_FIRST: c_uint = 0;
const MDB_NEXT: c_uint = 8;

#[repr(C)]
struct MdbValue {
    size: usize,
    data: *mut c_void,
}

#[repr(C)]
struct MdbEnv {
    _private: [u8; 0],
}

#[repr(C)]
struct MdbTxn {
    _private: [u8; 0],
}

#[repr(C)]
struct MdbCursor {
    _private: [u8; 0],
}

unsafe extern "C" {
    fn ztools_mdb_env_create(environment: *mut *mut MdbEnv) -> c_int;
    fn ztools_mdb_env_set_maxdbs(environment: *mut MdbEnv, databases: c_uint) -> c_int;
    fn ztools_mdb_env_open(
        environment: *mut MdbEnv,
        path: *const c_char,
        flags: c_uint,
        mode: c_uint,
    ) -> c_int;
    fn ztools_mdb_env_close(environment: *mut MdbEnv);
    fn ztools_mdb_txn_begin(
        environment: *mut MdbEnv,
        parent: *mut MdbTxn,
        flags: c_uint,
        transaction: *mut *mut MdbTxn,
    ) -> c_int;
    fn ztools_mdb_txn_abort(transaction: *mut MdbTxn);
    #[cfg(test)]
    fn ztools_mdb_txn_commit(transaction: *mut MdbTxn) -> c_int;
    fn ztools_mdb_dbi_open(
        transaction: *mut MdbTxn,
        name: *const c_char,
        flags: c_uint,
        database: *mut c_uint,
    ) -> c_int;
    fn ztools_mdb_dbi_close(environment: *mut MdbEnv, database: c_uint);
    fn ztools_mdb_cursor_open(
        transaction: *mut MdbTxn,
        database: c_uint,
        cursor: *mut *mut MdbCursor,
    ) -> c_int;
    fn ztools_mdb_cursor_close(cursor: *mut MdbCursor);
    fn ztools_mdb_cursor_get(
        cursor: *mut MdbCursor,
        key: *mut MdbValue,
        data: *mut MdbValue,
        operation: c_uint,
    ) -> c_int;
    #[cfg(test)]
    fn ztools_mdb_put(
        transaction: *mut MdbTxn,
        database: c_uint,
        key: *mut MdbValue,
        data: *mut MdbValue,
        flags: c_uint,
    ) -> c_int;
    fn ztools_mdb_strerror(error: c_int) -> *const c_char;
}

/// 使用项目原版 LMDB v2 ABI 只读遍历 main 命名数据库。
pub(crate) fn read_main_database(path: &Path) -> Result<Vec<(String, String)>, String> {
    let path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| "旧数据路径包含空字符".to_owned())?;
    let name = CString::new("main").expect("static database name is valid");
    let mut environment = ptr::null_mut();
    let mut transaction = ptr::null_mut();
    let mut cursor = ptr::null_mut();
    let mut database = 0;

    // 所有入口都使用 MDB_RDONLY | MDB_NOLOCK，保证不创建锁文件或修改源库。
    unsafe {
        check(ztools_mdb_env_create(&mut environment), "创建 LMDB 环境")?;
        let result = (|| {
            check(
                ztools_mdb_env_set_maxdbs(environment, 16),
                "设置 LMDB 数据库数量",
            )?;
            check(
                ztools_mdb_env_open(environment, path.as_ptr(), MDB_RDONLY | MDB_NOLOCK, 0),
                "打开 LMDB v2",
            )?;
            check(
                ztools_mdb_txn_begin(environment, ptr::null_mut(), MDB_RDONLY, &mut transaction),
                "开始 LMDB 只读事务",
            )?;
            check(
                ztools_mdb_dbi_open(transaction, name.as_ptr(), 0, &mut database),
                "打开 main 数据库",
            )?;
            check(
                ztools_mdb_cursor_open(transaction, database, &mut cursor),
                "打开 LMDB 游标",
            )?;
            read_cursor(cursor)
        })();

        // 逆序释放原生句柄；空指针表示对应阶段尚未成功。
        if !cursor.is_null() {
            ztools_mdb_cursor_close(cursor);
        }
        if !transaction.is_null() {
            ztools_mdb_txn_abort(transaction);
        }
        if !environment.is_null() {
            if database != 0 {
                ztools_mdb_dbi_close(environment, database);
            }
            ztools_mdb_env_close(environment);
        }
        result
    }
}

/// 从已定位的原生游标复制 UTF-8 键值，避免在事务结束后保留映射内存引用。
unsafe fn read_cursor(cursor: *mut MdbCursor) -> Result<Vec<(String, String)>, String> {
    let mut result = Vec::new();
    let mut key = MdbValue {
        size: 0,
        data: ptr::null_mut(),
    };
    let mut value = MdbValue {
        size: 0,
        data: ptr::null_mut(),
    };
    let mut operation = MDB_FIRST;
    loop {
        let code = unsafe { ztools_mdb_cursor_get(cursor, &mut key, &mut value, operation) };
        if code == MDB_NOTFOUND {
            break;
        }
        check(code, "遍历 LMDB v2")?;
        let key_bytes = unsafe { slice::from_raw_parts(key.data.cast::<u8>(), key.size) };
        let value_bytes = unsafe { slice::from_raw_parts(value.data.cast::<u8>(), value.size) };
        let key = std::str::from_utf8(key_bytes)
            .map_err(|error| format!("旧数据库键不是 UTF-8：{error}"))?
            .to_owned();
        let value = std::str::from_utf8(value_bytes)
            .map_err(|error| format!("旧数据库值不是 UTF-8：{error}"))?
            .to_owned();
        result.push((key, value));
        operation = MDB_NEXT;
    }
    Ok(result)
}

/// 将 LMDB 返回码转换为包含原生错误描述的 Rust 结果。
fn check(code: c_int, operation: &str) -> Result<(), String> {
    if code == 0 {
        return Ok(());
    }
    let message = unsafe {
        let pointer = ztools_mdb_strerror(code);
        if pointer.is_null() {
            format!("错误码 {code}")
        } else {
            CStr::from_ptr(pointer).to_string_lossy().into_owned()
        }
    };
    Err(format!("{operation}失败：{message}"))
}

#[cfg(test)]
/// 创建真实 LMDB v2 命名数据库，供只读兼容层和迁移测试使用。
pub(crate) fn write_test_database(path: &Path, entries: &[(&str, &str)]) -> Result<(), String> {
    const MDB_CREATE: c_uint = 0x40_000;
    std::fs::create_dir_all(path).map_err(|error| error.to_string())?;
    let path = CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| "测试数据库路径包含空字符".to_owned())?;
    let name = CString::new("main").expect("static database name is valid");
    let mut environment = ptr::null_mut();
    let mut transaction = ptr::null_mut();
    let mut database = 0;

    // 测试写入使用同一份 v2 C 实现，确保磁盘格式与 Electron 依赖一致。
    unsafe {
        check(
            ztools_mdb_env_create(&mut environment),
            "创建测试 LMDB 环境",
        )?;
        let result = (|| {
            check(
                ztools_mdb_env_set_maxdbs(environment, 16),
                "设置测试 LMDB 数据库数量",
            )?;
            check(
                ztools_mdb_env_open(environment, path.as_ptr(), 0, 0o600),
                "打开测试 LMDB v2",
            )?;
            check(
                ztools_mdb_txn_begin(environment, ptr::null_mut(), 0, &mut transaction),
                "开始测试 LMDB 写事务",
            )?;
            check(
                ztools_mdb_dbi_open(transaction, name.as_ptr(), MDB_CREATE, &mut database),
                "创建 main 数据库",
            )?;
            for (key, value) in entries {
                let mut key = MdbValue {
                    size: key.len(),
                    data: key.as_ptr().cast_mut().cast(),
                };
                let mut value = MdbValue {
                    size: value.len(),
                    data: value.as_ptr().cast_mut().cast(),
                };
                check(
                    ztools_mdb_put(transaction, database, &mut key, &mut value, 0),
                    "写入测试 LMDB 文档",
                )?;
            }
            check(ztools_mdb_txn_commit(transaction), "提交测试 LMDB 写事务")?;
            transaction = ptr::null_mut();
            Ok(())
        })();

        // 失败路径中止事务，成功路径只关闭数据库和环境。
        if !transaction.is_null() {
            ztools_mdb_txn_abort(transaction);
        }
        if !environment.is_null() {
            if database != 0 {
                ztools_mdb_dbi_close(environment, database);
            }
            ztools_mdb_env_close(environment);
        }
        result
    }
}
