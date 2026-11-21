# GitHub Copilot sign-in

An official Sailry sign-in package. A user signs in with GitHub's device code
and uses the models of their Copilot subscription in Sailry's own loop.

The package declares the flow; the execution Node performs it. The Node sends
the declared requests, shows the code, waits, stores the credential and renews
it. The flow script is a pure step function without network, filesystem or SDK
access.

1. The Node requests a device code from GitHub and shows the code with
   `https://github.com/login/device`.
2. It polls at GitHub's interval, slows down when asked, and stops when the
   code expires, the user declines or sign-in is cancelled.
3. It exchanges the GitHub token for a short-lived Copilot session token. The
   GitHub token is kept as the renewal credential, and the exchange is repeated
   shortly before the session expires.

Model requests, their headers, the service endpoint and model discovery belong
to the application's `copilot` authentication. This version serves github.com
accounts at `https://api.githubcopilot.com`; GitHub Enterprise Cloud with data
residency (`*.ghe.com`) is not supported.

The package signs in on its own. It does not read another application's
sign-in files or keychain items, and it does not show plan or quota status.

The session exchange is not a documented public interface. Synthetic fixtures
cover the wire behaviour; they do not prove access with a real account.
