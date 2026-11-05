PHONE AND SITE WORD AUDIT, CARD #558. Read only; nothing changed. All frontend paths are under frontend/src/.

**A. HOW THE PHONE IS BUILT**

- There is no native UI. The phone runs the same React frontend in a Tauri webview, from one build (docs/android.md:36, docs/ios.md:20,38). Fixing a shared string fixes it on the phone too.
- "Phone" is a width, not a platform:
  - `useNarrow` / `narrowQuery` is under 768px (lib/media.ts:5,24).
  - `useIsMobile` is used only in ui/sidebar.tsx (hooks/use-mobile.ts).
  - Tailwind `sm:` hides things under 640px.
  - So every phone path below also shows in a narrow desktop window. The 640-767px band gets the tabs and drawer but keeps the wide header buttons.
- `isAndroid()` is used only by the updater (updater/android.ts:16). No platform branch touches either word.
- Native files:
  - Android: strings.xml and all .kt files have no board/space words. The only "keyboard" hit is AndroidManifest.xml:23. NotifyPlugin.kt:47 "Mentions and direct messages" is fine.
  - iOS has one hit, listed in B9.
- Rust mobile: tray_mobile.rs is a no-op. notify_android.rs has no words.
- Phone notifications: `on_phone` passes only Talk and `msg:` ids (crates/notify/src/lib.rs:60-62). Titles such as "{from} sent you a message" (crates/inbox/src/watch.rs:778) have no words.
  - These do NOT reach the phone as notifications: "Open Board" (crates/notify/src/lib.rs:618), which is a test-note button, and the phone path sends only title and body (notify_android.rs:57-64).
  - The hold reasons at crates/inbox/src/watch.rs:228-231 also don't arrive as notifications, because held messages stay off the phone (docs/android.md:53). They are in hold_reason(), and I did not trace whether the Held page in the drawer displays them.
- The command palette opens only from a key (App.tsx:543-550), so a phone without a keyboard never reaches its "New board" (App.tsx:648) or "Rename this board" (boards/board-page.tsx:187).
- `title=` tooltips don't show on touch. On the phone, title strings matter less than visible text and aria-labels.

**B. PHONE-ONLY STRINGS AND PATHS (current -> proposed)**

1. components/phone-tabs.tsx:20, the third bottom tab. `label: "Boards"` -> `label: "Spaces"`.
   - Why: the page behind it lists spaces today, one tile per space (boards/boards-page.tsx:35,72). Each opens a space page.
   - Alternative: keep "Boards" only if the tab becomes the picked space's own lists. Then All spaces and Personal need a defined view. The label is true only if that lands first.
   - Layout path: phone-tabs.tsx:13 sends section kinds `boards` and `board` (lib/nav.ts:18-19) to this tab. If an inner board gets its own page (s3), it needs a new section kind and must map here too, or the tab highlight falls to Home at phone-tabs.tsx:14.
   - `kind: "board"` currently means a space page, and `#boards` is a hash route (lib/nav.ts:29). Those can stay internal, but the names will mislead.
2. phone-tabs.tsx comments:
   - :6 "Home, Talk or Boards" -> "Home, Talk or Spaces"
   - :7 type member `"boards"` -> `"spaces"` (optional, internal)
   - :9-10 "one board's page is under Boards" -> "a space's page and its boards are under Spaces"
   - :23 "Home, Talk and Boards" -> "Home, Talk and Spaces"
   - :25 "the Boards page" -> "the Spaces page"
3. App.tsx:712 comment "Home, Talk and Boards are tabs" -> "Home, Talk and Spaces are tabs". The bar renders at App.tsx:971-977.
4. components/app-header.tsx:88. Under 640px the button is only a plus icon (:91-92), so the aria-label is its whole name, and it is what a screen reader says.
   - `aria-label="New board"` -> `"New space"`
   - :89 `"Make a board for a project, or join one with an invite"` -> `"Make a space for a group of people, or join one with an invite"`
   - :92 `+ New board` -> `+ New space` (shown from 640px up)
5. boards/board-page.tsx:404-406, a Rename item that shows only under 640px (`sm:hidden`), inside the menu at :398. It is the phone's only rename route, because :270 hides the header button. It renames the space.
   - :405 `Rename` -> `Rename space`
   - :398 `aria-label="Board actions" title="Board actions"` -> `"Space actions"`
   - :229 `aria-label="Board name"` -> `"Space name"`
   - :351 comment "Rename for owners" is fine.
   - Once rename of an inner board exists (s3), the phone needs its own route for it. This menu has no room for it as it stands, and `SpaceBoards` chips have no menu at all.
6. boards/board-page.tsx:247, `hidden ... lg:inline`. The "N cards · visible to ..." line never shows on a phone, so the space page there doesn't say who is in it. No word to fix, but it is a gap against "space = people". Suggest showing the members on the phone's space page.
7. Gap, not a string: the picked space is visible on the phone only inside the drawer.
   - `SpaceSwitcher` sits only at app-sidebar.tsx:367.
   - The header `sub` line is hidden under md (app-header.tsx:63), and App.tsx passes no `sub` at all.
   - All three tabs filter by a space the screen doesn't name. The Talk badge counts by it too (App.tsx:716-722).
   - Suggest the header names the space on the phone, or the switcher moves to the tab pages. Mock this in s2.
8. Gap, not a string: the drawer repeats the tabs. Home, Talk and Boards appear in the drawer (app-sidebar.tsx:378-426) and in the bottom bar. If the tab becomes Spaces, the drawer item must change with it (see E).
9. app/gen/apple/project.yml:57 `Irori syncs your boards with your other devices on this network.` -> `Irori syncs your spaces with your other devices on this network.`
   - app/gen/apple/irori-app_iOS/Info.plist:24 has the same text. project.yml:40 names that plist as the target path and docs/ios.md:15 says to run `xcodegen generate` after editing project.yml. So edit project.yml and regenerate; both files are committed, so both change. I did not run xcodegen. If it doesn't update the plist, edit :24 by hand.
10. docs/android.md:3 `The phone build is a board client: ... syncs your cards and boards over iroh` -> `The phone build is a client for your spaces: ... syncs your spaces, boards and cards over iroh`
11. docs/ios.md:3 `it's a board client` -> `it's a client for your spaces`
12. Design docs: docs/design/spaces.md has no mention of the phone. Add a line on the tab when amending it. docs/design/talk-channels.md:80 and docs/design/merge-desk.md §8 (:170) have no board/space problem. docs/img/talk/talk-5.png is the phone mockup behind the tabs and likely shows the old tab name; I did not open it.

Phone-only paths checked, with no board/space words:
- Talk narrow branches (talk/talk-page.tsx:327,453,479-485,977 and talk/conversation.tsx:209-256).
- board-view.tsx narrow Threads sheet (cards/board-view.tsx:239-258,280-298,329).
- Drawer close and menu labels (app-sidebar.tsx:343, app-header.tsx:51).
- Canvas `!narrow` (canvas/reactflow/index.tsx:296).
- Settings full screen (settings/settings-dialog.tsx:131).
- Talk on the phone is scoped by space, not by board (`threadsSpace(space)`, App.tsx:272,849). A notification tap still opens a conversation from another space, because the open one is looked up across all spaces (talk/talk-page.tsx:142-147).

**C. SITE**

Only site/src/App.tsx is about the concept:

- :40 `title: 'Shared boards'` -> `title: 'Spaces'`
- :43 `A board you share with people you invite, around any topic: their agents work your shared cards alongside yours, through your home servers, and you decide what reaches your agents.` -> `A space for the people you invite, around any topic: its boards, Talk and threads are shared, their agents work your shared cards alongside yours, through your home servers, and you decide what reaches your agents.`
- :31 `one inbox and one Board`: no proposal yet. It means your own personal cards, so it depends on the open Personal blocker. The same phrase is at settings/settings-dialog.tsx:71 and cards/card-detail.tsx:686, so all three should move together.

Exclude these: Hero.tsx:53,65, index.css:149, scene/HeroScene.tsx and scene/layout.ts. There "board" is the 3D circuit-board picture, and HeroScene.tsx:625 "world space" is unrelated. site/index.html has no hit. site/public holds only logo.svg.

**D. README**

No hits. README.md is build and release notes only.

**E. SHARED SURFACES THE PHONE REACHES (references only; the window audit owns the text)**

Visible on the phone's main path:
- App.tsx:774 header title "Boards"
- boards/boards-page.tsx:41,43-45,64-65,70,79-80,83 (:41 duplicates the header title on the phone)
- boards/board-page.tsx:135,152,199,311-312,326-328,359,370,483-518
- boards/parts.tsx:123-144,186-197,284,442-490,620,692
- app-sidebar.tsx:420,425,449 ("Pinned boards" lists spaces)
- space-switcher.tsx:64-65 (:65 "Pick the space Home, Talk and Boards show" must follow the B1 decision)
- settings/settings-dialog.tsx:71,77,275,278,471-472
- settings/devices-panel.tsx:1112,1119
- profile/profile-page.tsx:259,300
- cards/card-grid.tsx:242,250,264,914
- cards/share-dialog.tsx:84,93,102,211
- cards/card-detail.tsx:686,743,1450
- cards/new-card-dialog.tsx:144-148
- cards/ask-box.tsx:66,71
- cards/bits.tsx:433
- home/spawn-notice.tsx:22
- app-header.tsx:102

Other findings for the plan:
- space-switcher.tsx has no New space or Join entry. Creating and joining live only in `NewBoardDialog` behind the Boards tab, so the tab's meaning decides where New space goes on the phone.
- Space names and the switcher's "All spaces" / "Personal" come from lib/spaces.ts:34-40. Personal's phone copy is boards-page.tsx:64-65, which waits on the Personal blocker.
- The `SpaceBoards` chips and add form (board-page.tsx:483-518) wrap on the phone but appear only after a second board or while adding one. "Every space opens on its boards" needs a real phone layout here, because `w-48` plus `w-24` inputs and two buttons is the tightest form on that page.
- Tests: no test pins the phone tab labels. Only cards/board-view.test.ts and lib/boards.test.ts contain the Boards/New board/Your boards text. I confirmed the board-view.test.ts hits are titles and scope values, not asserted UI strings; I did not inspect the lib/boards.test.ts hits.