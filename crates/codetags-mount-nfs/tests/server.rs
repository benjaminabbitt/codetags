//! The spike's NFSv3 server over real RPC, on every OS: no kernel mount, so
//! these run unprivileged anywhere (P0b S2).
#![allow(clippy::expect_used, clippy::unwrap_used, missing_docs)]

use std::net::{IpAddr, Ipv4Addr};

use codetags_mount_nfs::spike::{HELLO_CONTENT, HELLO_NAME, Server};
use nfs3_client::Nfs3Connection;
use nfs3_client::nfs3_types::nfs3::{
    GETATTR3args, GETATTR3res, LOOKUP3args, LOOKUP3res, Nfs3Result, READ3args, diropargs3, fattr3,
    nfs_fh3, nfsstat3,
};
use nfs3_client::tokio::{TokioConnector, TokioIo};
use tokio::net::TcpStream;

type Connection = Nfs3Connection<TokioIo<TcpStream>>;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

async fn connect(server: &Server, export_path: &str) -> Result<Connection, String> {
    nfs3_client::Nfs3ConnectionBuilder::new(TokioConnector, "127.0.0.1", export_path)
        .connect_from_privileged_port(false)
        .mount_port(server.port())
        .nfs3_port(server.port())
        .mount()
        .await
        .map_err(|error| format!("{error:?}"))
}

async fn lookup(conn: &mut Connection, name: &str) -> Result<nfs_fh3, nfsstat3> {
    let args = LOOKUP3args {
        what: diropargs3 {
            dir: conn.root_nfs_fh3(),
            name: name.as_bytes().into(),
        },
    };
    match conn.lookup(&args).await.unwrap() {
        LOOKUP3res::Ok(ok) => Ok(ok.object),
        LOOKUP3res::Err((status, _)) => Err(status),
    }
}

async fn getattr(conn: &mut Connection, object: nfs_fh3) -> fattr3 {
    match conn.getattr(&GETATTR3args { object }).await.unwrap() {
        GETATTR3res::Ok(ok) => ok.obj_attributes,
        GETATTR3res::Err((status, _)) => panic!("getattr: {status:?}"),
    }
}

async fn read_all(conn: &mut Connection, file: nfs_fh3) -> Vec<u8> {
    let args = READ3args {
        file,
        offset: 0,
        count: 4096,
    };
    match conn.read(&args).await.unwrap() {
        Nfs3Result::Ok(ok) => ok.data.to_vec(),
        Nfs3Result::Err((status, _)) => panic!("read: {status:?}"),
    }
}

fn start() -> Server {
    Server::start(501, 20).expect("start the spike server")
}

#[test]
fn listens_on_loopback_only() {
    let server = start();
    assert_eq!(server.ip(), IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_ne!(server.port(), 0);
    server.stop().unwrap();
}

#[test]
fn export_paths_are_unguessable() {
    let (a, b) = (start(), start());
    for server in [&a, &b] {
        let token = server.export_path().strip_prefix('/').unwrap();
        assert_eq!(token.len(), 32, "{token}");
        assert!(
            token.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{token}"
        );
    }
    assert_ne!(a.export_path(), b.export_path());
    a.stop().unwrap();
    b.stop().unwrap();
}

#[test]
fn serves_the_hello_file_at_the_export_path() {
    let server = start();
    runtime().block_on(async {
        let mut conn = connect(&server, server.export_path()).await.unwrap();
        let hello = lookup(&mut conn, HELLO_NAME).await.unwrap();
        let attr = getattr(&mut conn, hello.clone()).await;
        assert_eq!(attr.size, HELLO_CONTENT.len() as u64);
        assert_eq!((attr.uid, attr.gid), (501, 20));
        assert_eq!(attr.mode & 0o222, 0, "read-only");
        assert_eq!(read_all(&mut conn, hello).await, HELLO_CONTENT.as_bytes());
        assert_eq!(
            lookup(&mut conn, "absent.txt").await,
            Err(nfsstat3::NFS3ERR_NOENT)
        );
    });
    server.stop().unwrap();
}

#[test]
fn refuses_any_other_export_path() {
    let server = start();
    runtime().block_on(async {
        for path in ["/", "/codetags", &server.export_path()[..8]] {
            assert!(connect(&server, path).await.is_err(), "mounted {path}");
        }
    });
    server.stop().unwrap();
}

#[test]
fn an_added_file_is_served_and_the_root_mtime_moves_only_on_a_bump() {
    let server = start();
    runtime().block_on(async {
        let mut conn = connect(&server, server.export_path()).await.unwrap();
        let root = conn.root_nfs_fh3();
        let before = getattr(&mut conn, root.clone()).await.mtime;

        server.add_file("new.txt", b"added later", false);
        let added = lookup(&mut conn, "new.txt").await.unwrap();
        assert_eq!(read_all(&mut conn, added).await, b"added later");
        assert_eq!(getattr(&mut conn, root.clone()).await.mtime, before);

        server.add_file("newer.txt", b"bumped", true);
        let after = getattr(&mut conn, root).await.mtime;
        assert!(
            (after.seconds, after.nseconds) > (before.seconds, before.nseconds),
            "{before:?} -> {after:?}"
        );
    });
    server.stop().unwrap();
}
