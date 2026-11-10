package app.accesshub.resellerdemo;

import android.app.Activity;
import android.app.AlertDialog;
import android.content.Intent;
import android.graphics.Color;
import android.net.Uri;
import android.net.http.SslError;
import android.os.Bundle;
import android.view.Gravity;
import android.view.View;
import android.webkit.CookieManager;
import android.webkit.PermissionRequest;
import android.webkit.SslErrorHandler;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceError;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.io.ByteArrayInputStream;

public final class MainActivity extends Activity {
    private WebView web;
    private ValueCallback<Uri[]> files;
    private AlertDialog connectionDialog;

    static boolean allowed(Uri uri) { return uri != null && NavigationPolicy.allowed(uri.toString()); }

    @Override public void onCreate(Bundle state) {
        super.onCreate(state);
        LinearLayout layout = new LinearLayout(this);
        layout.setOrientation(LinearLayout.VERTICAL);
        layout.setBackgroundColor(Color.BLACK);
        layout.setOnApplyWindowInsetsListener((view, insets) -> {
            // Keep the independent native label below Android system bars.
            view.setPadding(insets.getSystemWindowInsetLeft(), insets.getSystemWindowInsetTop(),
                    insets.getSystemWindowInsetRight(), insets.getSystemWindowInsetBottom());
            return insets.consumeSystemWindowInsets();
        });
        TextView mark = new TextView(this);
        mark.setText("DEMO — NOT VALID ID");
        mark.setTextColor(Color.BLACK);
        mark.setBackgroundColor(0xffffdf32);
        mark.setTextSize(13);
        mark.setGravity(Gravity.CENTER);
        mark.setTypeface(null, 1);
        mark.setPadding(8, 12, 8, 12);
        layout.addView(mark, new LinearLayout.LayoutParams(-1, -2));
        web = new WebView(this);
        layout.addView(web, new LinearLayout.LayoutParams(-1, 0, 1));
        setContentView(layout);

        WebView.setWebContentsDebuggingEnabled(false);
        WebSettings settings = web.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);
        settings.setMixedContentMode(WebSettings.MIXED_CONTENT_NEVER_ALLOW);
        settings.setSafeBrowsingEnabled(true);
        settings.setSupportMultipleWindows(false);
        settings.setJavaScriptCanOpenWindowsAutomatically(false);
        settings.setMediaPlaybackRequiresUserGesture(true);
        settings.setUserAgentString(settings.getUserAgentString() + " AccessHubResellerDemo/1.0");
        CookieManager.getInstance().setAcceptCookie(true);
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, false);

        web.setWebViewClient(new WebViewClient() {
            @Override public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
                Uri url = request.getUrl();
                if (request.isForMainFrame() ? allowed(url) : NavigationPolicy.allowedFrame(url == null ? null : url.toString())) return false;
                if (request.isForMainFrame() && request.hasGesture() && "https".equals(url.getScheme())
                        && url.getUserInfo() == null && !NavigationPolicy.HOST.equals(url.getHost())) {
                    try { startActivity(new Intent(Intent.ACTION_VIEW, url)); }
                    catch (Exception ignored) { /* Retain the demo page if no browser is installed. */ }
                }
                return true;
            }
            @Override public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
                // Also cover main-frame form/POST navigations, for which WebView
                // does not always invoke shouldOverrideUrlLoading.
                if (request.isForMainFrame() && !allowed(request.getUrl())) {
                    return new WebResourceResponse("text/plain", "UTF-8", 403, "Forbidden",
                            java.util.Collections.emptyMap(), new ByteArrayInputStream(new byte[0]));
                }
                return null;
            }
            @Override public void onReceivedSslError(WebView view, SslErrorHandler handler, SslError error) { handler.cancel(); }
            @Override public void onReceivedError(WebView view, WebResourceRequest request, WebResourceError error) {
                if (!request.isForMainFrame() || isFinishing() || isDestroyed()
                        || (connectionDialog != null && connectionDialog.isShowing())) return;
                final String retry = NavigationPolicy.retryUrl(request.getUrl() == null ? null : request.getUrl().toString());
                connectionDialog = new AlertDialog.Builder(MainActivity.this)
                        .setTitle("Unable to connect")
                        .setMessage("Check your internet connection and try again.")
                        .setPositiveButton("Try again", (dialog, which) -> web.loadUrl(retry))
                        .setNegativeButton("Cancel", null).create();
                connectionDialog.show();
            }
        });
        web.setWebChromeClient(new WebChromeClient() {
            @Override public boolean onShowFileChooser(WebView view, ValueCallback<Uri[]> callback, FileChooserParams params) {
                if (files != null) files.onReceiveValue(null);
                files = callback;
                Intent pick = new Intent(Intent.ACTION_OPEN_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE)
                        .setType("image/*").putExtra(Intent.EXTRA_MIME_TYPES, new String[]{"image/png", "image/jpeg", "image/webp"});
                try { startActivityForResult(pick, 77); }
                catch (Exception error) { files.onReceiveValue(null); files = null; }
                return true;
            }
            @Override public void onPermissionRequest(PermissionRequest request) { request.deny(); }
            @Override public void onShowCustomView(View view, CustomViewCallback callback) {
                // A webpage fullscreen view must never cover the native label.
                callback.onCustomViewHidden();
            }
        });
        web.loadUrl(NavigationPolicy.START);
    }

    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == 77 && files != null) {
            Uri selected = result == RESULT_OK && data != null ? data.getData() : null;
            files.onReceiveValue(selected != null && "content".equals(selected.getScheme()) ? new Uri[]{selected} : null);
            files = null;
        }
    }
    @Override public void onBackPressed() { if (web.canGoBack()) web.goBack(); else super.onBackPressed(); }
    @Override protected void onResume() { super.onResume(); if (web != null) web.onResume(); }
    @Override protected void onPause() { if (web != null) web.onPause(); CookieManager.getInstance().flush(); super.onPause(); }
    @Override protected void onDestroy() {
        if (connectionDialog != null) connectionDialog.dismiss();
        if (files != null) { files.onReceiveValue(null); files = null; }
        if (web != null) web.destroy();
        super.onDestroy();
    }
}
