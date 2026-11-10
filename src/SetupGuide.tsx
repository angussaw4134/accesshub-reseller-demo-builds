export function SetupGuide({ platform }: { platform: string }) {
  return <details className="reseller-guide">
    <summary>Fictional demo setup · start here</summary>
    <p>This loader installs only the marked AccessHub reseller demonstration app. It is not valid identification and must not be used as proof of identity or age.</p>
    <ol>
      <li>Activate this computer using your private demo key or the personal installer you downloaded.</li>
      <li>Connect and unlock your iPhone. Tap Trust if your phone asks, then select the connected device.</li>
      <li>Sign in with your own Apple Account inside this installer. Follow any verification prompts from Apple.</li>
      <li>Choose the available fictional demo release, then Install. Keep the phone connected until the process finishes.</li>
      <li>If iOS asks, enable Developer Mode in Settings → Privacy &amp; Security and trust the developer in Settings → General → VPN &amp; Device Management.</li>
    </ol>
    {platform === 'windows' && <p>Windows: if the phone is not detected, check the cable and Apple’s device drivers before trying again.</p>}
    <p>Apps installed with a free Apple Account need regular renewal. Keep your private key and customer link for the next installation.</p>
    <p>Release availability and branding refresh while this loader is open. A desktop program update still requires a newly published installer.</p>
  </details>;
}
