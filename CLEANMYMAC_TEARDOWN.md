# CleanMyMac 5 — Teardown

Target: `/Applications/CleanMyMac_5.app` · v5.7.2 (build 50702.0.2610141002) · bundle `com.macpaw.CleanMyMac5` · team `S8EX82NJP6` · universal (x86_64 + arm64) · min macOS 11 · built with Xcode 26.6 / SDK 26.5 · 436 MB.

Method: static inspection only (bundle layout, Info.plist, entitlements, code-signing requirements, Swift/ObjC symbol tables, embedded strings, localizations), plus the app's own logs in its group container. No code was patched and nothing was decrypted. Licensing/activation was deliberately left out of scope.

Legend: **[obs]** observed directly in the bundle or logs · **[inf]** inferred from names or strings, not traced through code.

---

## 1. Process architecture

```
CleanMyMac_5 (main app, sandbox-less, Developer ID)
├── Frameworks/  ~130 in-house Swift frameworks (one per feature/layer)
├── Library/LaunchServices/com.macpaw.CleanMyMac5.Agent   ← privileged root helper (SMJobBless)
├── Library/LoginItems/
│   ├── CleanMyMac_5_Menu.app          ← menu-bar app (SMAppService login item)
│   └── CleanMyMac_5_HealthMonitor.app ← background monitor (SMAppService login item)
├── PlugIns/CleanMyMac_5_FinderSyncExtension.appex  ← Finder context menu / badges
├── Extensions/CleanMyMac_5_AppIntentsExtension.appex ← Shortcuts/Siri
├── XPCServices/…MASUpdaterXPCService.xpc            ← Mac App Store app updater
└── MacOS/CleanMyMac_5_Updater.app + Sparkle.framework ← self-update
```

- **[obs] Privileged helper.** `com.macpaw.CleanMyMac5.Agent` has an embedded `__launchd_plist` with `MachServices` and `SMAuthorizedClients`. It only accepts clients whose code-signing requirement is `identifier "com.macpaw.CleanMyMac5"|".Menu"|".HealthMonitor" and anchor apple generic and certificate leaf[subject.OU] = "S8EX82NJP6"`. The log line `PrivilegedAgent 🧐 The installation is successful` shows it being installed on the first scan that needs root.
- **[obs] IPC.** The `IPCProtocol` framework defines `XPCServerEngine`/`XPCClientEngine`, `XPCCredentials` and `ClientRequirement`, with a single raw entry point `sendWithRawMessage:reply:`. SwiftProtobuf is bundled. **[inf]** Messages are serialized blobs (likely protobuf) sent over one XPC method, and they aren't an NSXPC protocol per operation.
- **[obs] Shared state** goes through the app group `S8EX82NJP6.com.macpaw.CleanMyMac5` (also `.CLI` and the legacy `CleanMyMac4` group). It lives at `~/Library/Group Containers/S8EX82NJP6.com.macpaw.CleanMyMac5/` and holds `ExtendedStatistics.db`, `IssueIgnoreList.db`, `KnowledgeBase/`, `PantherDB/<ver>/` (malware signatures), `FeaturesCache/` (GrowthBook A/B flags) and per-process logs (CocoaLumberjack).

## 2. Code layout (framework naming convention)

Every feature is split into layers. That makes it a good pattern to copy:

| Layer | Examples |
|---|---|
| `*Scanning` (find things) | SystemJunkScanning, TrashJunkScanning, MailJunkScanning, PrivacyScanning, PerformanceScanning, UninstallerScanning, UpdaterScanning, SpaceLensScanning, MalwareScanning, CloudStorageScanning, DownloadsScanning, UnusedDiskImagesScanning, AIToolScanning, LocalModelScanning, OrganizeScanning |
| `*Module` (feature coordinator) | JunkCleanupModule, ProtectionModule, PerformanceModule, ApplicationsModule, SpaceLensModule, SmartCareModule, CloudStorageModule, OrganizeModule, EmailCleanupModule, ToolboxModule, AssistantModule |
| `*Service` (side effects, OS APIs) | FileManagerService, PrivacyService, PerformanceService, ApplicationsService, PrivilegedOperationsPerformerService, PantherEngineService, NotificationsService |
| Core engine | `ScanningCore`, `CleaningCore`, `NodesCore`, `ModuleCore`, `LegacyScanningCore` |
| UI | `*UI`, DesignSystem, SnapKit, RiveRuntime (animations), Kingfisher |
| Infra | DatabaseKit + FMDB + GRDB (SQLite), PreferencesStorage, RemotePreferences, MPTweaks, LoggerKit |

### ScanningCore: task graph **[obs]**
Types: `CMMTask`, `TypedTask`, `SingleValueTask`, `DependentTask`, `TaskDescriptor`, `TaskPriorityKind`, `TaskWatcher` (+ per-module timeouts from A/B flag CMY-5028), `GenericScanner`, `CommonModuleScanner`.

Each `*Scanning` framework exposes a `*ScanTasksFactory` that emits tasks. The log shows how they run:
```
[ResolveDependenciesTask #1] Started → (installs privileged agent, checks permissions) → Ended 5.67s
[Dependent<SafariBrowsingHistoryScanTask> #2] Started
[Dependent<MalwareScanTask> #5] Started
[Dependent<ChromeCookiesTask> #6] Started
[NetworkScanTask #10] Started
[Dependent<PrivacyFolderScanTask<PrivacyFileModel>> #11] Started …
```
A root dependency-resolution task runs first. After that, dozens of `Dependent<…>` scan tasks run concurrently, each one producing typed models.

### CleaningCore: removal pipeline **[obs]**
`RootNodeCleaner` → `AnyNodeCleaner` / `OperationBasedNodeCleaner` walk the result tree (`NodesCore`). They feed `FilesRemovalTask` through a `TasksProcessor`. Note that `MinimumTotalDurationTasksProcessor` exists; **[inf]** it pads the duration so the cleanup animation doesn't flash.
The remover strategies are pluggable:
- `FileRemover` (direct delete)
- `MoveToTrashFileRemover`
- `PrivilegedFileRemover` (routes through the Agent and throws `NotAuthorizedPrivilegedAgentError`)
- `ConfigDependentFileRemover` / `ConfigDependentRemover` (picks a strategy based on settings)

Results come back as `CleanTaskOperationResult` with `CMMTaskSize`/`CMMTaskCount` for the "X GB cleaned" summary.

## 3. Feature → mechanism map

### System Junk (`SystemJunkScanning`) **[obs types/strings]**
| Group (UI) | Model / remover | Notes |
|---|---|---|
| User / System Cache Files | `CacheModel`, `CacheFileModel` | Locations mostly come from the encrypted KB (§5). Hard-coded extras include Spotify `PersistentCache/Storage`, Adobe `Media Cache Files`, Telegram/WeChat/QQMusic/Youku/Tencent Video containers, and the Photos `Photos Desktop/0` cache |
| User / System Log Files | `LogsModel`, `LogsRemover` | also `<container>/Data/Library/Logs` |
| Language Files | `ApplicationLanguageModel/GroupModel` | unused `.lproj` inside app bundles |
| Universal Binaries | `UniversalBinaryModel`, `UniversalBinaryThinner` | The Agent has `getUnnecessaryThinFilesForBinaryAtPath:compatibleArchitectures:` / `removeUnnecessaryThinFilesFromBinaryAtPath:` and strips the non-native slice. It detects Electron via `Contents/Frameworks/Electron Framework.framework` |
| Broken Preferences | `BrokenPreferencesModel/Remover` | unparsable plists |
| Broken Login Items | `BrokenStartupItemModel/Remover` | launch agents/daemons whose target binary is gone |
| Document Versions | `DocumentVersionsApplicationModel` | `.DocumentRevisions-V100` |
| Deleted Users | `DeletedUserModel` | leftover home dirs (`/Library/SystemMigration/History`, `dscl`) |
| iOS Device Backups | `MobileBackupsModel` | `MobileSync/Backup` |
| Xcode Junk | `XcodeJunkModel`, `XcodeSimulatorRemover`, `XcodeSimulatorRuntimeRemover`, `XcodeDownloadedRuntimeModel`, `XcodeJunkDeviceLogsModel`, `XcodeJunkCoreSimulatorCacheModel` | Shells out to `xcrun simctl`, `xcode-select -p` and `.../CoreSimulator.framework/.../bin/simctl` |
| Quarantine history | (privacy) | `~/Library/Preferences/com.apple.LaunchServices.QuarantineEventsV2` |

### Trash Bins (`TrashJunkScanning`)
`TrashFileModel` (~/.Trash), `VolumeTrashModel` (`/Volumes/*/.Trashes`), `MailTrashModel` + `MailTrashAttachmentModel`.

### Mail Attachments (`MailJunkScanning`)
`MailAppJunkModel.AttachmentModel` (Apple Mail downloads) and `SparkJunkModel` (Readdle Spark).

### AI Clutter (new in 5.x) (`AIToolScanning`, `LocalModelScanning`)
- `AgentFootprintScanner`/`AIJunkScanTask` targets: Claude (`com.anthropic.claudefordesktop`, `claude-cli-nodejs`, `Library/Logs/Claude`), Codex (`com.openai.codex`, `codex-browser-use`), Cursor, Gemini/Antigravity, and `uv` caches.
- `LocalModelsScanTask` targets: Ollama (`~/.ollama/models`, `models/manifests`, `models/blobs`).

### Performance (`PerformanceScanning` / `PerformanceService`) **[obs]**
Maintenance tasks (`PerformanceTaskIdentifier`), plus **[inf]** the likely underlying command for each:

| Task | Likely implementation |
|---|---|
| `freeUpRAM` | `purge` / memory pressure |
| `freeUpPurgeableSpace` | creates a temporary large file to force APFS to evict purgeable data **[inf]** |
| `flushDNSCache` | `dscacheutil -flushcache` + `killall -HUP mDNSResponder` (both binaries referenced in the Agent) |
| `reindexSpotlight` | `mdutil -E /` (+ `spotlightIndexingObserver`) |
| `periodicScripts` | `/usr/sbin/periodic daily weekly monthly` |
| `repairDiskPermissions` | `diskutil resetUserPermissions / <uid>` (string `Failed to launch diskutil resetUserPermissions` in Agent) |
| `speedUpMail` | reindex Mail "Envelope Index" SQLite (needs Full Disk Access) |
| `thinLocalSnapshots` | `tmutil listlocalsnapshots` / `tmutil thinlocalsnapshots` |

There are also Login Items (`launchServicesLoginItems`, `LoginItemsKnowledgeBase`) and Background Items (`launchAgents`, `launchDaemonsPath`, `systemLaunchAgentsPath`). The Agent removes items through `unloadLaunchAgentWithConfigPath:`, `removeAgentAtPath:`, `removePrivilegedToolWithLabel:`, `removeMatchedCronjobWithSource:userName:` (`/usr/lib/cron/tabs`, `crontab`) and `profiles` / `mcxrefresh` (configuration profiles).

### Privacy (`PrivacyScanning`)
Factories: `SafariScanTaskFactory`, `ChromeScanTaskFactory`, `FirefoxScanTaskFactory` (reads `profiles.ini`), `RecentItemsScanTaskFactory`, `NetworkScanTaskFactory` (Wi-Fi networks), `ApplicationsPermissionScanTaskFactory` (reads `TCC.db`).
Tasks seen in the log cover browsing history, downloads history, cookies, local storage, autofill, saved passwords, search engines and tabs from the last session.

### Applications (`UninstallerScanning`, `UpdaterScanning`)
- Leftover search templates: `~/Library/{Application Support,Caches,Preferences,Logs,Saved Application State,Cookies/%@.binarycookies,Containers/,Application Support/CrashReporter}`, `/Library/{Application Support,Logs,PDF Services,Documentation/Applications}`, `/usr/local/bin`, Avid plug-ins, Adobe AIR ELS. The search matches on bundle ID and app name.
- `ApplicationContainerAccessProbeTask` checks whether it can read sandbox containers.
- Updater: `ApplicationsUpdatesScanTask` (Sparkle feeds + MAS via the XPC service) and `SystemUpdateScanTask`.

### Protection (malware)
`MalwareScanning`, `PANEngine`, `PantherEngineService` and the Moonlock* frameworks. The signature DB is `PantherDB/<version>` in the group container and updates at launch (the log shows 5.5.8 → 5.5.98). The Agent includes `PANFreezeDetector`.

### Other modules
Space Lens (disk tree map, plus a cloud strategy), Cloud Storage (iCloud/Dropbox/Google Drive/OneDrive local-vs-cloud items), Email Cleanup (Gmail/Microsoft via OAuth), Organize/Downloads/Unused Disk Images (`.dmg` files left in Downloads), Health Monitor (menu bar: CPU, RAM, disk, battery, Bluetooth devices), and Assistant/TipsServiceAI (LLM-backed tips; it bundles `schemas_for_prompt_results.json`).

## 4. Privileged Agent: operation surface **[obs selectors]**

File ops:
- `removeFileWithPath:leaveEmptyDirectories:`
- `securelyRemoveFileAtPath:` (`SecureRemoveErrorDomain`)
- `removeFileAndHandleLockAtPath:` (clears the uchg/immutable flag)
- `advancedMoveItemAtPath:toPath:`
- `removeQuarantineAttributeWithFileURL:`
- `removeExpandedContentWithURL:`

Process/launchd:
- `killProcessWithIdentifier:`
- `unloadLaunchAgentWithConfigPath:`
- `removeAgentAtPath:`
- `removePrivilegedToolWithLabel:`
- `removeJobsForUser:` / `removeMatchedJob:`

Binary thinning: `get/removeUnnecessaryThinFilesForBinaryAtPath:…`

External tools: `launchctl`, `crontab`, `defaults`, `dscacheutil`, `dscl`, `hdiutil`, `killall`, `mcxrefresh`, `profiles`, `diskutil`, `periodic`, `pkgutil`, `mdutil`. They run through a `taskRunner` that enforces `externalUtilityExecutionTimeout`.

Directory Services (`ODSession`/`ODNode`/`ODQuery`) are used to enumerate real users when detecting deleted-user leftovers.

## 5. The rule database ("Knowledge Base")

- `Resources/info.cmmkb` (298 KB) and `userInfo.cmmkb` ship in the bundle and are copied to `GroupContainer/.../KnowledgeBase/`. They're updated remotely and checked by SHA-256 hash (`KnowledgeBaseUpdater`).
- Byte entropy is 7.9994 bits/byte, so the files are **encrypted/compressed**. The safe-to-delete cache and log locations and the per-app rules live here, which is why few cache paths appear as plain strings in the binaries.
- I didn't try to decrypt it. It's MacPaw's proprietary curated data. For macscrap, build your own rule set (see §6).

## 6. Takeaways for building macscrap

1. **Architecture worth copying:** split each feature into Scanning (pure, returns typed models), Module (state machine `idle → scanning → results → cleaning → done`), and Service (side effects). Use a small task-graph runner with dependencies, concurrency, per-task timeouts and progress.
2. **Result tree plus remover strategies:** model results as a node tree with selectable leaves. Pick a remover per node (trash / direct / privileged), and make move-to-Trash the default.
3. **Root helper:** use `SMAppService.daemon` (macOS 13+) rather than deprecated SMJobBless. Validate callers with a code-signing requirement on the XPC connection (`setCodeSigningRequirement` on macOS 13+), expose a *narrow* command set, and never accept an arbitrary path to `rm -rf` without an allow-list.
4. **Rules as data:** keep cleaning locations in a versioned JSON/SQLite rule file (path glob, category, min age, needs-root, safety level) so you can update rules without shipping a new binary.
5. **Free wins with no rule DB:** `~/Library/Caches/*`, `~/Library/Logs`, Xcode `DerivedData`/`Archives`/`iOS DeviceSupport`/simulators (`xcrun simctl delete unavailable`), `~/.Trash`, `~/Library/Containers/*/Data/Library/Caches`, unused `.lproj`, dev caches (npm/pnpm/yarn/pip/uv/Homebrew/Cargo/Gradle), `~/.ollama/models`, and leftover `.dmg` files in Downloads.
6. **Permissions:** most scans need Full Disk Access. CleanMyMac probes with `ApplicationContainerAccessProbeTask` and shows an FDA resolution flow (`Features_FDAResolutionUIKit.bundle`).

## 7. Unknowns / not traced
- Exact XPC message schema (protobuf message definitions weren't recovered).
- Exact command lines for Free Up RAM and Purgeable Space (only names and descriptions were observed).
- KB rule format (encrypted, not attempted).
- Malware engine internals (PANEngine).
