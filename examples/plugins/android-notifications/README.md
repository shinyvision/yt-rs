# Android notifications prototype

The successful prototype has been integrated into the real
[notifications plugin](../notifications). It now uses the existing bell,
with embedded Google sign-in, keyring storage, automatic renewal, and the
Android inbox and unseen count. The standalone prototype code was retired.

Run `bash tests/manual/android-notifications.sh` for an isolated profile
using the real app and notification bell. It reuses that test profile's
existing keyring credential, or offers sign-in inside the bell.
