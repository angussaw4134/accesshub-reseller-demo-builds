package app.accesshub.resellerdemo;

import java.net.URI;
import java.net.URISyntaxException;
import java.util.regex.Pattern;

/** Routes served only by the permanently marked fictional demo. */
final class NavigationPolicy {
    static final String HOST = "3377speedincreese.fejiofasj.workers.dev";
    static final String START = "https://" + HOST + "/reseller-app";
    private static final Pattern TENANT = Pattern.compile("/r/partner-[a-f0-9]{12}/(?:loader|index(?:\\.html)?|access-entry(?:\\.html)?|faq(?:\\.html)?|demo-account\\.html)?");
    private static final Pattern FRAME = Pattern.compile("/r/partner-[a-f0-9]{12}/Home%20page/favourites-carousel\\.html");
    private NavigationPolicy() {}

    static boolean allowed(String value) {
        if (value == null || value.length() > 8192) return false;
        try {
            URI uri = new URI(value);
            if (!"https".equals(uri.getScheme()) || !HOST.equals(uri.getHost())
                    || uri.getRawUserInfo() != null || (uri.getPort() != -1 && uri.getPort() != 443)
                    || uri.getRawQuery() != null || uri.getRawFragment() != null) return false;
            // Raw-path matching rejects encoded separators and dot traversal.
            String path = uri.getRawPath();
            return "/reseller-app".equals(path) || "/reseller-app/".equals(path)
                    || (path != null && TENANT.matcher(path).matches());
        } catch (URISyntaxException error) { return false; }
    }
    static String retryUrl(String candidate) { return allowed(candidate) ? candidate : START; }
    static boolean allowedFrame(String value) {
        if (allowed(value)) return true;
        if (value == null || value.length() > 8192) return false;
        try {
            URI uri = new URI(value);
            return "https".equals(uri.getScheme()) && HOST.equals(uri.getHost()) && uri.getRawUserInfo() == null
                    && (uri.getPort() == -1 || uri.getPort() == 443) && uri.getRawPath() != null
                    && FRAME.matcher(uri.getRawPath()).matches() && uri.getRawFragment() == null
                    && (uri.getRawQuery() == null || uri.getRawQuery().matches("v=[0-9]{1,6}"));
        } catch (URISyntaxException error) { return false; }
    }
}
