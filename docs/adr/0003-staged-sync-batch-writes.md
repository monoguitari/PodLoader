# Staged Sync Batch Model for Device Mutations

All additions of Host Files, removals of Device Tracks, metadata edits, and playlist modifications are staged in memory as a `Sync Batch` rather than mutating the Device filesystem immediately on each user interaction. Committing a Sync Batch copies/transcodes audio into `iPod_Control/Music/Fxx/`, backs up the existing `iTunesDB`, and atomically writes the updated `iTunesDB` to minimize slow random I/O on mechanical iPod hard drives and prevent database corruption if disconnected mid-session.
