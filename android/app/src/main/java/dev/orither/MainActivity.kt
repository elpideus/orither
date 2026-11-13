package dev.orither

import android.Manifest
import android.content.Intent
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.orither.link.LinkService
import dev.orither.ui.Root

class MainActivity : ComponentActivity() {
    private var deepLink by mutableStateOf<String?>(null)

    private val askForNotifications =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) {}

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        askForNotifications.launch(Manifest.permission.POST_NOTIFICATIONS)
        LinkService.start(this)
        deepLink = intent.dataString
        setContent { Root(deepLink, onDeepLinkUsed = { deepLink = null }) }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        deepLink = intent.dataString
    }
}
