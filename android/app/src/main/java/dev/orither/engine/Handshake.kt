package dev.orither.engine

import java.io.DataInputStream
import java.net.Socket
import java.security.MessageDigest
import java.security.SecureRandom
import javax.crypto.Mac
import javax.crypto.spec.SecretKeySpec

/**
 * Every loopback connection between the app and the engine starts here. Both
 * prove they hold the secret without revealing it, because any app on the phone
 * could connect to the port, or sit on it before the engine starts.
 */
internal object Handshake {
    private const val SIZE = 32

    /** Runs the engine's side. Returns false when the other end is not the app. */
    fun asEngine(socket: Socket, secret: ByteArray): Boolean {
        val input = DataInputStream(socket.getInputStream())
        val output = socket.getOutputStream()
        val challenge = nonce()
        output.write(challenge)
        output.flush()
        val answer = ByteArray(SIZE).also(input::readFully)
        val theirChallenge = ByteArray(SIZE).also(input::readFully)
        if (!MessageDigest.isEqual(answer, proof(secret, "app", challenge))) return false
        output.write(proof(secret, "engine", theirChallenge))
        output.flush()
        return true
    }

    /** Runs the app's side. Returns false when the other end is not the engine. */
    fun asApp(socket: Socket, secret: ByteArray): Boolean {
        val input = DataInputStream(socket.getInputStream())
        val output = socket.getOutputStream()
        val theirChallenge = ByteArray(SIZE).also(input::readFully)
        val challenge = nonce()
        output.write(proof(secret, "app", theirChallenge))
        output.write(challenge)
        output.flush()
        val answer = ByteArray(SIZE).also(input::readFully)
        return MessageDigest.isEqual(answer, proof(secret, "engine", challenge))
    }

    fun nonce(): ByteArray = ByteArray(SIZE).also(SecureRandom()::nextBytes)

    private fun proof(secret: ByteArray, role: String, challenge: ByteArray): ByteArray =
        Mac.getInstance("HmacSHA256").run {
            init(SecretKeySpec(secret, algorithm))
            update(role.toByteArray())
            doFinal(challenge)
        }
}
