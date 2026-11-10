import Foundation

let host = "https://3377speedincreese.fejiofasj.workers.dev"
let tenant = "/r/partner-0123456789ab/"
let accepted = ["/reseller-app", "/reseller-app/", tenant, tenant + "loader", tenant + "access-entry.html", tenant + "faq.html", tenant + "demo-account.html", tenant + "index.html"]
for path in accepted {
    precondition(allowedDemoNavigation(URL(string: host + path)!), "Allowed route rejected: \(path)")
}
let rejected = ["/", "/admin", "/reseller-app/resolve", "/reseller-app/../", "/reseller-app?key=secret", tenant + "?migrationAdmin=1", tenant + "#secret", "/r/other/", "/r/partner-0123456789ab", tenant + "../../", tenant + "%2e%2e/%2e%2e/", tenant + "%2F..%2Fadmin", tenant + "%252e%252e/", tenant + "%5c..%5cadmin", tenant + "/index.html", tenant + "api/admin/login", tenant + "Home%20page/favourites-carousel.html?v=5"]
for path in rejected {
    if let url = URL(string: host + path) {
        precondition(!allowedDemoNavigation(url), "Unsafe route accepted: \(path)")
    }
}
for address in ["http://3377speedincreese.fejiofasj.workers.dev/reseller-app", "https://3377speedincreese.fejiofasj.workers.dev.evil.example/reseller-app", "https://user@3377speedincreese.fejiofasj.workers.dev/reseller-app", "https://3377speedincreese.fejiofasj.workers.dev:444/reseller-app", "file:///reseller-app", "javascript:alert(1)"] {
    precondition(!allowedDemoNavigation(URL(string: address)!), "Unsafe origin accepted: \(address)")
}
precondition(allowedDemoNavigation(URL(string: host + tenant + "Home%20page/favourites-carousel.html?v=5")!, mainFrame: false))
precondition(!allowedDemoNavigation(URL(string: host + tenant + "Home%20page/favourites-carousel.html?redirect=/")!, mainFrame: false))
print("PASS: iOS navigation permits fictional demo documents and its scoped favourites frame; rejects wrong origins, owner routes, traversal, encoded separators, raw key URLs and arbitrary subpages.")
