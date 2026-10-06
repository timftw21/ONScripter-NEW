//! Android entry point and owned document-provider file descriptors.
#![cfg(target_os = "android")]

use jni::{
    EnvUnowned, JavaVM, jni_sig, jni_str,
    objects::{Global, JObject, JString, JValue},
};
use onscripter_core::{
    Error, Limits, Result,
    assets::{AssetStore, ReadSeek, Storage},
    script::{Program, ScriptSource},
};
use std::{
    ffi::{c_char, c_int},
    fs::File,
    io,
    os::fd::FromRawFd,
};

struct Documents {
    vm: JavaVM,
    activity: Global<JObject<'static>>,
}

impl Storage for Documents {
    fn open(&self, name: &str) -> io::Result<Option<Box<dyn ReadSeek>>> {
        let descriptor = self
            .vm
            .attach_current_thread(|env| -> jni::errors::Result<(i32, Option<String>)> {
                let name = JString::from_str(env, name)?;
                let result = env.call_method(
                    self.activity.as_obj(),
                    jni_str!("openAssetFd"),
                    jni_sig!("(Ljava/lang/String;)I"),
                    &[JValue::Object(name.as_ref())],
                );
                if result.is_err() {
                    env.exception_describe();
                    env.exception_clear();
                }
                let descriptor = result?.i()?;
                let error = if descriptor == -2 {
                    let value = env.call_method(
                        self.activity.as_obj(),
                        jni_str!("getAssetError"),
                        jni_sig!("()Ljava/lang/String;"),
                        &[],
                    )?;
                    Some(JString::cast_local(env, value.l()?)?.to_string())
                } else {
                    None
                };
                Ok((descriptor, error))
            })
            .map_err(io::Error::other)?;
        let (descriptor, error) = descriptor;
        if let Some(error) = error {
            return Err(io::Error::other(error));
        }
        if descriptor == -1 {
            return Ok(None);
        }
        if descriptor < 0 {
            return Err(io::Error::other(
                "document provider returned an invalid file handle",
            ));
        }
        // GameDocuments.detachFd transfers a unique owned descriptor. File closes
        // it exactly once, including when a decoder or archive parser fails.
        let file = unsafe { File::from_raw_fd(descriptor) };
        Ok(Some(Box::new(file)))
    }
}

fn run(storage: Documents) -> Result<()> {
    let limits = Limits::default();
    let mut source = None;
    for name in ["script.file", "0.txt"] {
        if let Some(mut reader) = storage.open(name)? {
            source = Some(ScriptSource::read(&mut reader, limits)?);
            break;
        }
    }
    let source = source.ok_or_else(|| {
        Error::invalid("chosen game folder contains neither script.file nor 0.txt")
    })?;
    let program = Program::parse(source, limits)?;
    onscripter_render::player::play(
        &program,
        AssetStore::new(storage, limits),
        Default::default(),
    )
}

/// SDLActivity invokes this on SDL's attached native engine thread.
#[unsafe(no_mangle)]
pub extern "C" fn SDL_main(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    // SDL supplies the current thread's JNIEnv and a fresh local Activity ref.
    // JNI's with_env catches panics, preventing unwinding across the C boundary.
    let raw = unsafe { sdl3_sys::system::SDL_GetAndroidJNIEnv() };
    if raw.is_null() {
        return 1;
    }
    let mut unowned = unsafe { EnvUnowned::from_raw(raw.cast()) };
    unowned
        .with_env(|env| -> jni::errors::Result<c_int> {
            let raw_activity = unsafe { sdl3_sys::system::SDL_GetAndroidActivity() };
            if raw_activity.is_null() {
                return Ok(1);
            }
            let activity = unsafe { JObject::from_raw(env, raw_activity.cast()) };
            let storage = Documents {
                vm: env.get_java_vm()?,
                activity: env.new_global_ref(&activity)?,
            };
            match run(storage) {
                Ok(()) => Ok(0),
                Err(error) => {
                    let message = JString::from_str(env, error.to_string())?;
                    env.call_method(
                        &activity,
                        jni_str!("showEngineError"),
                        jni_sig!("(Ljava/lang/String;)V"),
                        &[JValue::Object(message.as_ref())],
                    )?;
                    Ok(1)
                }
            }
        })
        .resolve::<jni::errors::ThrowRuntimeExAndDefault>()
}
