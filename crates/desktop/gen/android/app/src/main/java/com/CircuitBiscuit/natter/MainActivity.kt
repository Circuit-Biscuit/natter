package com.CircuitBiscuit.natter

import android.os.Build
import android.os.Bundle
import android.webkit.JavascriptInterface
import android.webkit.WebView
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }

  // Runs before the first page load, so `window.NatterAndroid` is there from the start.
  override fun onWebViewCreate(webView: WebView) {
    webView.addJavascriptInterface(AppInfo(this), "NatterAndroid")
  }
}

/** What the app (glue.js `androidInstaller`) may ask the Android side. Kept by proguard-rules.pro. */
class AppInfo(private val activity: MainActivity) {
  /** The store that installed Natter: "com.android.vending" for Google Play, "" for a sideloaded APK. */
  @JavascriptInterface
  fun installer(): String = try {
    val pm = activity.packageManager
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
      pm.getInstallSourceInfo(activity.packageName).installingPackageName
    } else {
      @Suppress("DEPRECATION")
      pm.getInstallerPackageName(activity.packageName)
    } ?: ""
  } catch (e: Exception) {
    ""
  }
}
