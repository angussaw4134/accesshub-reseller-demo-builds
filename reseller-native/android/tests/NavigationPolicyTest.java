package app.accesshub.resellerdemo;

public final class NavigationPolicyTest {
    public static void main(String[] args) {
        String host = "https://3377speedincreese.fejiofasj.workers.dev";
        String[] allowed = {host + "/reseller-app", host + "/reseller-app/", host + "/r/partner-0123456789ab/index",
                host + "/r/partner-0123456789ab/", host + "/r/partner-0123456789ab/access-entry.html",
                host + "/r/partner-0123456789ab/loader", host + "/r/partner-0123456789ab/index.html",
                host + "/r/partner-0123456789ab/faq.html", host + "/r/partner-0123456789ab/demo-account.html",
                host + "/r/partner-0123456789ab/access-entry", host + "/r/partner-0123456789ab/faq",
                "https://3377speedincreese.fejiofasj.workers.dev:443/reseller-app"};
        for (String value : allowed) if (!NavigationPolicy.allowed(value)) throw new AssertionError("Blocked safe route: " + value);
        String[] blocked = {null, "", host, host + "/", host + "/license", host + "/reseller-app-legacy",
                host + "/reseller-app/../../", host + "/r/partner-0123456789ab/../../", host + "/r/partner-0123456789ab/%2e%2e/%2e%2e/",
                host + "/r/partner-0123456789ab/%252e%252e/", host + "/r/partner-0123456789ab/%2f%2e%2e/",
                host + "/r/partner-0123456789ab/\\..\\", host + "/r/not-a-partner/", host + "/r/partner-0123456789ab/old-id",
                host + "/r/partner-0123456789ab//loader", host + "/r/partner-0123456789ab/loader/../",
                host + "/reseller-app/resolve", host + "/r/partner-0123456789ab/api/login", host + "/reseller-app#private-key",
                host + "/r/partner-0123456789ab/loader?key=secret", host + "/reseller-app?migrationAdmin=1",
                host + "/r/partner-0123456789ab/?demo=0", host + "/r/partner-0123456789ab/#key",
                "http://3377speedincreese.fejiofasj.workers.dev/reseller-app", "javascript:alert(1)", "file:///reseller-app",
                "content://3377speedincreese.fejiofasj.workers.dev/reseller-app", "https://attacker.test/reseller-app",
                "https://3377speedincreese.fejiofasj.workers.dev.attacker.test/reseller-app",
                "https://user@3377speedincreese.fejiofasj.workers.dev/reseller-app",
                "https://3377speedincreese.fejiofasj.workers.dev:444/reseller-app"};
        for (String value : blocked) if (NavigationPolicy.allowed(value)) throw new AssertionError("Allowed unsafe route: " + value);
        if (!NavigationPolicy.START.equals(NavigationPolicy.retryUrl(null))) throw new AssertionError("Null retry is not safe");
        if (!NavigationPolicy.START.equals(NavigationPolicy.retryUrl(host + "/legacy"))) throw new AssertionError("Invalid retry is not safe");
        if (!allowed[3].equals(NavigationPolicy.retryUrl(allowed[3]))) throw new AssertionError("Valid retry lost tenant");
        String frame = host + "/r/partner-0123456789ab/Home%20page/favourites-carousel.html?v=5";
        if (!NavigationPolicy.allowedFrame(frame) || NavigationPolicy.allowed(frame)) throw new AssertionError("Iframe scope is wrong");
        if (NavigationPolicy.allowedFrame(frame + "&redirect=/")) throw new AssertionError("Iframe query bypass");
        if (NavigationPolicy.allowedFrame(frame + "#key")) throw new AssertionError("Iframe fragment bypass");
        for (String value : blocked) if (NavigationPolicy.allowedFrame(value)) throw new AssertionError("Allowed unsafe iframe route: " + value);
        System.out.println("Navigation policy passed " + (allowed.length + blocked.length * 2 + 6) + " checks.");
    }
}
