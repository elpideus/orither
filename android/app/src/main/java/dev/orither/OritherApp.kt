package dev.orither

import android.app.Application
import dev.orither.feature.Features

class OritherApp : Application() {
    override fun onCreate() {
        super.onCreate()
        Features.install(this)
    }
}
