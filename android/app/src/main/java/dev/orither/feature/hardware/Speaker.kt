package dev.orither.feature.hardware

import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioTrack
import android.os.ParcelFileDescriptor
import dev.orither.feature.Feature
import dev.orither.link.LinkStream
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.serialization.json.JsonElement

private const val SAMPLE_RATE = 48_000
private const val READ_SIZE = 3840

/** Plays what the computer sends: 16-bit stereo at 48 kHz. */
internal class Speaker : Feature {
    override val id = "speaker"

    override suspend fun stream(method: String, body: JsonElement, stream: LinkStream, peer: String) {
        val socket = stream.socket()
        withContext(Dispatchers.IO) {
            val track = track()
            track.play()
            ParcelFileDescriptor.AutoCloseInputStream(socket).use { sound ->
                val buffer = ByteArray(READ_SIZE)
                while (true) {
                    val count = sound.read(buffer)
                    if (count < 0) break
                    track.write(buffer, 0, count)
                }
            }
            track.stop()
            track.release()
        }
    }

    private fun track(): AudioTrack {
        val format = AudioFormat.Builder()
            .setSampleRate(SAMPLE_RATE)
            .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
            .setChannelMask(AudioFormat.CHANNEL_OUT_STEREO)
            .build()
        val attributes = AudioAttributes.Builder()
            .setUsage(AudioAttributes.USAGE_MEDIA)
            .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
            .build()
        return AudioTrack.Builder()
            .setAudioFormat(format)
            .setAudioAttributes(attributes)
            .setPerformanceMode(AudioTrack.PERFORMANCE_MODE_LOW_LATENCY)
            .setTransferMode(AudioTrack.MODE_STREAM)
            .build()
    }
}
