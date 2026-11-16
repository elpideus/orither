//! The `external fun`s of `dev.orither.link.Native`.

use std::{
    os::fd::{FromRawFd, OwnedFd},
    path::PathBuf,
};

use anyhow::{Context, Result};
use jni::{
    Env, EnvUnowned,
    errors::ThrowRuntimeExAndDefault,
    jni_str,
    objects::{Global, JObject, JString},
    strings::JNIString,
    sys::{jboolean, jint, jlong},
};
use orither_link::install_android_jni_context;
use serde_json::{Value, json};

use crate::bridge::Bridge;

type Local<'a> = JObject<'a>;

/// Runs the body of a native function, turning errors into Java exceptions.
fn guarded<'local, T: Default>(
    env: &mut EnvUnowned<'local>,
    body: impl FnOnce(&mut Env<'local>) -> Result<T>,
) -> T {
    env.with_env(|env| -> Result<T, jni::errors::Error> {
        body(env).map_err(|err| {
            let message = JNIString::from(format!("{err:#}"));
            let _ = env.throw_new(jni_str!("java/lang/IllegalStateException"), message);
            jni::errors::Error::JavaException
        })
    })
    .resolve::<ThrowRuntimeExAndDefault>()
}

fn text(env: &Env<'_>, value: &JString<'_>) -> Result<String> {
    Ok(value.try_to_string(env)?)
}

fn json_body(env: &Env<'_>, value: &JString<'_>) -> Result<Value> {
    serde_json::from_str(&text(env, value)?).context("bad JSON body")
}

fn keep(env: &Env<'_>, object: &JObject<'_>) -> Result<Global<JObject<'static>>> {
    Ok(env.new_global_ref(object)?)
}

/// Takes ownership of a descriptor detached from a `ParcelFileDescriptor`.
fn owned(fd: jint) -> OwnedFd {
    // SAFETY: Kotlin hands over a descriptor it detached, so nothing else owns it.
    unsafe { OwnedFd::from_raw_fd(fd) }
}

/// Starts the node and returns this device's id.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_start<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    context: Local<'local>,
    data_dir: JString<'local>,
    name: JString<'local>,
    listener: Local<'local>,
) -> JString<'local> {
    guarded(&mut env, |env| {
        let _ = tracing_android_init();
        let vm = env.get_java_vm()?;
        // Lets the network layer ask Android for the DNS servers in use.
        let context = std::mem::ManuallyDrop::new(keep(env, &context)?);
        // SAFETY: the reference to the context above is never released.
        unsafe { install_android_jni_context(vm.get_raw().cast(), context.as_raw().cast()) };
        let listener = keep(env, &listener)?;
        let dir = PathBuf::from(text(env, &data_dir)?);
        let bridge = Bridge::start(vm, listener, dir, text(env, &name)?)?;
        Ok(env.new_string(bridge.node.id().to_string())?)
    })
}

fn tracing_android_init() -> Result<()> {
    use tracing_subscriber::{filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt};

    tracing_subscriber::registry()
        .with(LevelFilter::INFO)
        .with(tracing_android::layer("Orither")?)
        .try_init()?;
    Ok(())
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_peers<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
) -> JString<'local> {
    guarded(&mut env, |env| {
        let peers = serde_json::to_string(&Bridge::get()?.node.peers())?;
        Ok(env.new_string(peers)?)
    })
}

/// Starts accepting a new device and returns the code and QR payload as JSON.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_invite<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
) -> JString<'local> {
    guarded(&mut env, |env| {
        let bridge = Bridge::get()?;
        let invite = {
            let _entered = bridge.runtime.enter();
            bridge.node.invite()
        };
        Ok(env.new_string(serde_json::to_string(&invite)?)?)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_cancelInvite<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
) {
    guarded(&mut env, |_| {
        Bridge::get()?.node.cancel_invite();
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_join<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    token: JString<'local>,
    reply: Local<'local>,
) {
    guarded(&mut env, |env| {
        let bridge = Bridge::get()?;
        let token = text(env, &token)?;
        let reply = keep(env, &reply)?;
        bridge.answer(reply, async move {
            let peer = bridge.node.join(&token).await?;
            Ok(serde_json::to_value(peer)?)
        });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_unpair<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    peer: JString<'local>,
) {
    guarded(&mut env, |env| {
        let peer = Bridge::peer(&text(env, &peer)?)?;
        Bridge::get()?.node.unpair(&peer)
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_networkChanged<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
) {
    guarded(&mut env, |_| {
        let bridge = Bridge::get()?;
        bridge.runtime.spawn(bridge.node.network_changed());
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_call<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    peer: JString<'local>,
    feature: JString<'local>,
    method: JString<'local>,
    body: JString<'local>,
    reply: Local<'local>,
) {
    guarded(&mut env, |env| {
        let bridge = Bridge::get()?;
        let peer = Bridge::peer(&text(env, &peer)?)?;
        let (feature, method) = (text(env, &feature)?, text(env, &method)?);
        let body = json_body(env, &body)?;
        let reply = keep(env, &reply)?;
        bridge.answer(reply, async move {
            let session = bridge.node.session(&peer).context("device is offline")?;
            session.call(&feature, &method, body).await
        });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_notify<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    peer: JString<'local>,
    feature: JString<'local>,
    method: JString<'local>,
    body: JString<'local>,
) {
    guarded(&mut env, |env| {
        let bridge = Bridge::get()?;
        let peer = Bridge::peer(&text(env, &peer)?)?;
        let session = bridge.node.session(&peer).context("device is offline")?;
        session.notify(&text(env, &feature)?, &text(env, &method)?, json_body(env, &body)?)
    });
}

/// Answers an incoming call with JSON, or with an error when `ok` is false.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_respond<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    handle: jlong,
    ok: jboolean,
    body: JString<'local>,
) {
    guarded(&mut env, |env| {
        let result = if ok { Ok(json_body(env, &body)?) } else { Err(text(env, &body)?) };
        Bridge::get()?.respond(handle as u64, result);
        Ok(())
    });
}

/// Opens a stream to a peer. The reply holds a stream handle.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_open<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    peer: JString<'local>,
    feature: JString<'local>,
    method: JString<'local>,
    body: JString<'local>,
    reply: Local<'local>,
) {
    guarded(&mut env, |env| {
        let bridge = Bridge::get()?;
        let peer = Bridge::peer(&text(env, &peer)?)?;
        let (feature, method) = (text(env, &feature)?, text(env, &method)?);
        let body = json_body(env, &body)?;
        let reply = keep(env, &reply)?;
        bridge.answer(reply, async move {
            let session = bridge.node.session(&peer).context("device is offline")?;
            let stream = session.open(&feature, &method, body).await?;
            Ok(json!(bridge.keep_stream(stream)))
        });
        Ok(())
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_pump<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    handle: jlong,
    socket: jint,
) {
    guarded(&mut env, |_| Bridge::get()?.pump(handle as u64, owned(socket)));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_pumpVideo<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    handle: jlong,
    socket: jint,
) {
    guarded(&mut env, |_| Bridge::get()?.pump_video(handle as u64, owned(socket)));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_addNearby<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    socket: jint,
    reply: Local<'local>,
) {
    guarded(&mut env, |env| {
        let reply = keep(env, &reply)?;
        Bridge::get()?.add_nearby(owned(socket), reply)
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_sendFile<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    handle: jlong,
    file: jint,
) {
    guarded(&mut env, |_| Bridge::get()?.send_file(handle as u64, owned(file)));
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_receiveFile<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    handle: jlong,
    file: jint,
    reply: Local<'local>,
) {
    guarded(&mut env, |env| {
        let reply = keep(env, &reply)?;
        Bridge::get()?.receive_file(handle as u64, owned(file), reply)
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_orither_link_Native_dropStream<'local>(
    mut env: EnvUnowned<'local>,
    _this: Local<'local>,
    handle: jlong,
) {
    guarded(&mut env, |_| {
        Bridge::get()?.drop_stream(handle as u64);
        Ok(())
    });
}
