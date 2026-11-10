import Foundation

private let demoHost = "3377speedincreese.fejiofasj.workers.dev"
private let start = URL(string: "https://\(demoHost)/reseller-app")!
func allowedDemoNavigation(_ url: URL, mainFrame: Bool = true) -> Bool {
    guard url.scheme == "https", url.host == demoHost, url.user == nil, url.password == nil,
          url.port == nil || url.port == 443,
          let components = URLComponents(url: url, resolvingAgainstBaseURL: false) else { return false }
    let path = components.percentEncodedPath
    guard !path.contains("\\"), !path.contains("//"),
          !path.split(separator: "/", omittingEmptySubsequences: false).contains(where: { $0 == "." || $0 == ".." }) else { return false }
    // Only the known fictional documents may navigate inside the native shell.
    // APIs/assets are subresource requests; they do not need a navigation bypass.
    if path == "/reseller-app" || path == "/reseller-app/" {
        return components.query == nil && components.fragment == nil
    }
    let document = "^/r/partner-[a-f0-9]{12}/(?:|loader|access-entry(?:\\.html)?|faq(?:\\.html)?|demo-account\\.html|index(?:\\.html)?)$"
    if path.range(of: document, options: .regularExpression) != nil {
        return components.query == nil && components.fragment == nil
    }
    if !mainFrame {
        let favourites = "^/r/partner-[a-f0-9]{12}/Home%20page/favourites-carousel\\.html$"
        return path.range(of: favourites, options: .regularExpression) != nil
            && (components.query == nil || components.query?.range(of: "^v=[0-9]{1,6}$", options: .regularExpression) != nil)
            && components.fragment == nil
    }
    return false
}

#if canImport(UIKit)
import SwiftUI
import WebKit
import UIKit

@main struct ResellerDemoApp: App {
    var body: some Scene {
        WindowGroup {
            VStack(spacing: 0) {
                Text("DEMO — NOT VALID ID").font(.system(size: 13, weight: .heavy)).frame(maxWidth: .infinity).padding(9).background(Color.yellow).foregroundColor(.black)
                DemoBrowser()
            }.background(Color.black).preferredColorScheme(.dark)
        }
    }
}
struct DemoBrowser: UIViewRepresentable {
    func makeCoordinator() -> Coordinator { Coordinator() }
    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .default()
        let web = WKWebView(frame: .zero, configuration: config)
        web.navigationDelegate = context.coordinator
        web.uiDelegate = context.coordinator
        web.isOpaque = false
        web.backgroundColor = .black
        web.allowsBackForwardNavigationGestures = true
        web.customUserAgent = "Mozilla/5.0 (iPhone; CPU iPhone OS 16_0 like Mac OS X) AppleWebKit/605.1.15 Mobile AccessHubResellerDemo/1.0"
        web.load(URLRequest(url: start))
        return web
    }
    func updateUIView(_ view: WKWebView, context: Context) {}
    final class Coordinator: NSObject, WKNavigationDelegate, WKUIDelegate {
        func webView(_ webView: WKWebView, decidePolicyFor action: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            guard let url = action.request.url else { decisionHandler(.cancel); return }
            if allowedDemoNavigation(url, mainFrame: action.targetFrame?.isMainFrame ?? true) {
                decisionHandler(.allow); return
            }
            decisionHandler(.cancel)
            if action.navigationType == .linkActivated && ["https", "mailto", "tel"].contains(url.scheme ?? "") && url.host != demoHost {
                UIApplication.shared.open(url)
            }
        }
        func webView(_ webView: WKWebView, decidePolicyFor response: WKNavigationResponse, decisionHandler: @escaping (WKNavigationResponsePolicy) -> Void) {
            guard let url = response.response.url, allowedDemoNavigation(url, mainFrame: response.isForMainFrame) else {
                decisionHandler(.cancel); return
            }
            decisionHandler(.allow)
        }
        func webView(_ webView: WKWebView, createWebViewWith configuration: WKWebViewConfiguration, for action: WKNavigationAction, windowFeatures: WKWindowFeatures) -> WKWebView? {
            // Script-created windows never open another app or bypass the shell.
            guard action.navigationType == .linkActivated, let url = action.request.url else { return nil }
            if allowedDemoNavigation(url) { webView.load(URLRequest(url: url)) }
            else if url.scheme == "https", url.host != demoHost, url.user == nil, url.password == nil {
                UIApplication.shared.open(url)
            }
            return nil
        }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            guard (error as NSError).code != NSURLErrorCancelled else { return }
            let alert = UIAlertController(title: "Unable to connect", message: "Check your internet connection, then try again.", preferredStyle: .alert)
            alert.addAction(UIAlertAction(title: "Try again", style: .default) { _ in webView.load(URLRequest(url: webView.url.flatMap { allowedDemoNavigation($0) ? $0 : nil } ?? start)) })
            alert.addAction(UIAlertAction(title: "Cancel", style: .cancel))
            webView.window?.rootViewController?.present(alert, animated: true)
        }
    }
}
#endif
