package dev.orither.ui

import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.lifecycle.awaitInstance
import androidx.camera.view.PreviewView
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BinaryBitmap
import com.google.zxing.NotFoundException
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader

private const val PAIRING_PREFIX = "orither://pair/"

/** A live camera that reports the first Orither pairing code it sees. */
@Composable
internal fun Scanner(found: (String) -> Unit, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    val owner = LocalLifecycleOwner.current
    val view = remember { PreviewView(context).apply { implementationMode = PreviewView.ImplementationMode.COMPATIBLE } }

    LaunchedEffect(Unit) {
        val camera = ProcessCameraProvider.awaitInstance(context)
        val preview = Preview.Builder().build().apply { surfaceProvider = view.surfaceProvider }
        val analysis = ImageAnalysis.Builder()
            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
            .build()
        val reader = QRCodeReader()
        analysis.setAnalyzer(ContextCompat.getMainExecutor(context)) { frame ->
            frame.use { read(reader, it) }?.takeIf { it.startsWith(PAIRING_PREFIX) }?.let(found)
        }
        camera.unbindAll()
        camera.bindToLifecycle(owner, CameraSelector.DEFAULT_BACK_CAMERA, preview, analysis)
    }
    AndroidView({ view }, modifier)
}

private fun read(reader: QRCodeReader, frame: ImageProxy): String? {
    val plane = frame.planes.first()
    val bytes = ByteArray(plane.buffer.remaining()).also(plane.buffer::get)
    val source = PlanarYUVLuminanceSource(bytes, plane.rowStride, frame.height, 0, 0, frame.width, frame.height, false)
    return try {
        reader.decode(BinaryBitmap(HybridBinarizer(source))).text
    } catch (_: NotFoundException) {
        null
    } catch (_: ReaderException) {
        null
    } finally {
        reader.reset()
    }
}
