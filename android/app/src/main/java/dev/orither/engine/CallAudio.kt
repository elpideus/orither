package dev.orither.engine

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.os.Build
import java.io.InputStream
import java.io.OutputStream
import java.net.Socket
import kotlin.concurrent.thread

/** Call sound travels as 16-bit mono at 16 kHz both ways: wideband speech, and light on the link. */
internal const val CALL_RATE = 16_000
private const val CALL_CHUNK = CALL_RATE / 50 * 2

private val CALL_FORMAT_IN: AudioFormat = AudioFormat.Builder()
    .setSampleRate(CALL_RATE).setEncoding(AudioFormat.ENCODING_PCM_16BIT).setChannelMask(AudioFormat.CHANNEL_IN_MONO).build()
private val CALL_FORMAT_OUT: AudioFormat = AudioFormat.Builder()
    .setSampleRate(CALL_RATE).setEncoding(AudioFormat.ENCODING_PCM_16BIT).setChannelMask(AudioFormat.CHANNEL_OUT_MONO).build()

/**
 * Carries a call's sound between the phone and a computer, over [client]: what the
 * other person says goes out, and what comes in is said into the call. Only the
 * shell user may do this, which is why it lives in the engine.
 *
 * - A mobile call is heard through the modem's own downlink, and spoken into through
 *   the in-call music path to the telephony uplink, with the phone's microphone muted.
 * - An app's call (WhatsApp, Telegram…) is taken over with an audio policy, as
 *   Android's own app streaming does: the app's playback goes to the computer instead
 *   of the earpiece, and the computer's microphone replaces the phone's in the app.
 */
internal class CallAudio(private val context: Context) {
    private val audio = context.getSystemService(AudioManager::class.java)

    /** Everything set up and checked, so that a failure is told before any sound flows. */
    inner class Prepared(private val record: AudioRecord, private val track: AudioTrack, private val undo: () -> Unit) {
        fun carry(client: Socket) {
            try {
                carry(client, record, track)
            } finally {
                undo()
            }
        }
    }

    fun cellular(): Prepared {
        check(audio.mode == AudioManager.MODE_IN_CALL) { "There is no phone call going on" }
        val uplink = audio.getDevices(AudioManager.GET_DEVICES_OUTPUTS).firstOrNull { it.type == AudioDeviceInfo.TYPE_TELEPHONY }
            ?: error("This phone has no way to speak into a call")
        val record = record(MediaRecorder.AudioSource.VOICE_DOWNLINK)
        val track = track(AudioAttributes.USAGE_MEDIA).apply { setPreferredDevice(uplink) }
        // The person on the other end hears the computer, not the phone's own microphone too.
        val muted = audio.isMicrophoneMute
        audio.isMicrophoneMute = true
        return Prepared(record, track) { audio.isMicrophoneMute = muted }
    }

    fun app(uid: Int): Prepared {
        val policy = Policy(context, uid)
        return Prepared(policy.record, policy.track, policy::release)
    }

    private fun record(source: Int): AudioRecord {
        val size = maxOf(AudioRecord.getMinBufferSize(CALL_RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT), CALL_CHUNK * 4)
        val builder = AudioRecord.Builder().setAudioSource(source).setAudioFormat(CALL_FORMAT_IN).setBufferSizeInBytes(size)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) builder.setContext(context)
        return builder.build().also { check(it.state == AudioRecord.STATE_INITIALIZED) { "The call's sound can't be heard on this phone" } }
    }

    @SuppressLint("NewApi")
    private fun track(usage: Int): AudioTrack {
        val attributes = AudioAttributes.Builder().setUsage(usage).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build()
        val builder = AudioTrack.Builder()
            .setAudioAttributes(attributes)
            .setAudioFormat(CALL_FORMAT_OUT)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setBufferSizeInBytes(CALL_CHUNK * 4)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) builder.setContext(context)
        return builder.build()
    }

    /** Pumps both ways until the computer hangs up the stream, or the call's sound ends. */
    private fun carry(client: Socket, record: AudioRecord, track: AudioTrack) {
        record.startRecording()
        track.play()
        val speaking = thread(isDaemon = true) {
            runCatching { pump(client.getInputStream(), track) }
            runCatching { client.shutdownInput() }
        }
        try {
            runCatching { listen(record, client.getOutputStream()) }
        } finally {
            runCatching { client.shutdownOutput() }
            speaking.join(1_000)
            record.stop()
            record.release()
            track.stop()
            track.release()
        }
    }

    private fun listen(record: AudioRecord, output: OutputStream) {
        val buffer = ByteArray(CALL_CHUNK)
        while (true) {
            val count = record.read(buffer, 0, buffer.size)
            if (count < 0) return
            if (count > 0) output.write(buffer, 0, count)
        }
    }

    private fun pump(input: InputStream, track: AudioTrack) {
        val buffer = ByteArray(CALL_CHUNK)
        while (true) {
            val count = input.read(buffer)
            if (count < 0) return
            // Whole samples only, so a split read never shifts the bytes of what follows.
            track.write(buffer, 0, count)
        }
    }
}

/**
 * The audio policy that takes over one app's call: a loop-back mix of what the app
 * plays, and an injector mix for what it records. These are system APIs, so they are
 * reached by reflection; the shell holds the permissions they ask for.
 */
@SuppressLint("PrivateApi", "DiscouragedPrivateApi", "SoonBlockedPrivateApi")
private class Policy(context: Context, uid: Int) {
    private val audio = context.getSystemService(AudioManager::class.java)
    private val policy: Any
    val record: AudioRecord
    val track: AudioTrack

    init {
        val players = mix(rule(uid, injector = false), CALL_FORMAT_OUT, captureVoice = true)
        val recorders = mix(rule(uid, injector = true), CALL_FORMAT_OUT, captureVoice = false)
        val builder = Class.forName("android.media.audiopolicy.AudioPolicy\$Builder")
            .getConstructor(Context::class.java).newInstance(context)
        val mixClass = Class.forName("android.media.audiopolicy.AudioMix")
        builder.javaClass.getMethod("addMix", mixClass).invoke(builder, players)
        builder.javaClass.getMethod("addMix", mixClass).invoke(builder, recorders)
        policy = builder.javaClass.getMethod("build").invoke(builder)!!
        val registered = AudioManager::class.java.getMethod("registerAudioPolicy", policy.javaClass).invoke(audio, policy) as Int
        check(registered == 0) { "Android didn't let Orither take over the call's sound ($registered)" }
        record = policy.javaClass.getMethod("createAudioRecordSink", mixClass).invoke(policy, players) as AudioRecord
        track = policy.javaClass.getMethod("createAudioTrackSource", mixClass).invoke(policy, recorders) as AudioTrack
    }

    fun release() {
        runCatching { AudioManager::class.java.getMethod("unregisterAudioPolicy", policy.javaClass).invoke(audio, policy) }
    }

    /** Matches one app: its players, or, as an injector, its recorders. */
    private fun rule(uid: Int, injector: Boolean): Any {
        val type = Class.forName("android.media.audiopolicy.AudioMixingRule\$Builder")
        val builder = type.getConstructor().newInstance()
        type.getMethod("setTargetMixRole", Int::class.javaPrimitiveType).invoke(builder, if (injector) MIX_ROLE_INJECTOR else MIX_ROLE_PLAYERS)
        type.getMethod("addMixRule", Int::class.javaPrimitiveType, Any::class.java).invoke(builder, RULE_MATCH_UID, uid)
        if (!injector) {
            // The call itself is voice communication, which only privileged capture may take.
            runCatching { type.getMethod("allowPrivilegedPlaybackCapture", Boolean::class.javaPrimitiveType).invoke(builder, true) }
            runCatching { type.getMethod("voiceCommunicationCaptureAllowed", Boolean::class.javaPrimitiveType).invoke(builder, true) }
        }
        return type.getMethod("build").invoke(builder)!!
    }

    private fun mix(rule: Any, format: AudioFormat, captureVoice: Boolean): Any {
        val type = Class.forName("android.media.audiopolicy.AudioMix\$Builder")
        val builder = type.getConstructor(rule.javaClass).newInstance(rule)
        type.getMethod("setFormat", AudioFormat::class.java).invoke(builder, format)
        type.getMethod("setRouteFlags", Int::class.javaPrimitiveType).invoke(builder, ROUTE_FLAG_LOOP_BACK)
        if (captureVoice) runCatching { type.getMethod("setVoiceCommunicationCaptureAllowed", Boolean::class.javaPrimitiveType).invoke(builder, true) }
        return type.getMethod("build").invoke(builder)!!
    }

    private companion object {
        // From android.media.audiopolicy, which the public SDK does not include.
        const val RULE_MATCH_UID = 1 shl 2
        const val MIX_ROLE_PLAYERS = 0
        const val MIX_ROLE_INJECTOR = 1
        const val ROUTE_FLAG_LOOP_BACK = 1 shl 1
    }
}
