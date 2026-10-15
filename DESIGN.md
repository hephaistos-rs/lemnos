---
name: Lemnos
description: Forge your own cloud. A self-service console for machines, drawn as bronze at the forge.
colors:
  bronze: "light-dark(#c98232, #cd8a3e)"
  bronze-hover: "light-dark(#b8721f, #dd9d54)"
  on-bronze: "#1b1004"
  bronze-text: "light-dark(#8a4d12, #e2a660)"
  patina: "light-dark(#1f7468, #62b8a8)"
  shell: "light-dark(#0f4a45, #0a3532)"
  shell-raised: "light-dark(#17605a, #114943)"
  shell-ink: "#e6f1ec"
  shell-muted: "#a9cdc3"
  danger: "light-dark(#b3351f, #c4452f)"
  on-danger: "#ffffff"
  danger-text: "light-dark(#a32b18, #f39480)"
  warning-text: "light-dark(#7a5a00, #e6c15a)"
  heat-1: "#4fae9d"
  heat-2: "#c8853a"
  heat-3: "#e6c15a"
  heat-4: "#d0603a"
  heat-5: "#8a5fb8"
  ground: "light-dark(#e6eeea, #0b1c1b)"
  surface: "light-dark(#f4f8f6, #10292a)"
  sunken: "light-dark(#d8e3de, #163536)"
  field: "light-dark(#ffffff, #0b1c1b)"
  line: "light-dark(#bccbc5, #24494a)"
  line-strong: "light-dark(#6f8a83, #4a7d7a)"
  ink: "light-dark(#0d2523, #e8efe9)"
  ink-muted: "light-dark(#3f5d58, #a8c0b9)"
  ink-faint: "light-dark(#4f6963, #84a09a)"
typography:
  title:
    fontFamily: "Commissioner, ui-sans-serif, system-ui, sans-serif"
    fontSize: "1.5rem"
    fontWeight: 600
    lineHeight: "2rem"
  lead:
    fontFamily: "Commissioner, ui-sans-serif, system-ui, sans-serif"
    fontSize: "1.125rem"
    fontWeight: 600
    lineHeight: "1.5rem"
  base:
    fontFamily: "Commissioner, ui-sans-serif, system-ui, sans-serif"
    fontSize: "0.9375rem"
    fontWeight: 400
    lineHeight: "1.5rem"
  meta:
    fontFamily: "Commissioner, ui-sans-serif, system-ui, sans-serif"
    fontSize: "0.8125rem"
    fontWeight: 500
    lineHeight: "1.25rem"
  figure:
    fontFamily: "JetBrains Mono, ui-monospace, monospace"
    fontSize: "0.8125rem"
    fontWeight: 400
    lineHeight: "1.25rem"
    fontFeature: "tnum"
  display:
    fontFamily: "Commissioner, ui-sans-serif, system-ui, sans-serif"
    fontSize: "clamp(2.25rem, 1.4rem + 3.6vw, 4rem)"
    fontWeight: 700
    lineHeight: 1.05
    letterSpacing: "-0.02em"
  wordmark:
    fontFamily: "Commissioner, ui-sans-serif, system-ui, sans-serif"
    fontSize: "1.125rem"
    fontWeight: 700
    lineHeight: "1.5rem"
    letterSpacing: "-0.01em"
  wordmark-greek:
    fontFamily: "Commissioner, ui-sans-serif, system-ui, sans-serif"
    fontSize: "0.8125rem"
    fontWeight: 500
    lineHeight: "1.25rem"
    letterSpacing: "0.16em"
rounded:
  control: "0.25rem"
spacing:
  field: "0.375rem"
  cell: "0.75rem"
  gutter: "1rem"
  gutter-wide: "1.5rem"
  panel: "1.25rem"
  panel-wide: "1.5rem"
  section: "2rem"
  row: "2.5rem"
  bar: "3.5rem"
components:
  button:
    backgroundColor: "{colors.sunken}"
    textColor: "{colors.ink}"
    typography: "{typography.base}"
    rounded: "{rounded.control}"
    padding: "0.375rem 0.875rem"
  button-hover:
    backgroundColor: "{colors.line}"
  button-primary:
    backgroundColor: "{colors.bronze}"
    textColor: "{colors.on-bronze}"
    rounded: "{rounded.control}"
    padding: "0.375rem 0.875rem"
  button-primary-hover:
    backgroundColor: "{colors.bronze-hover}"
  button-danger:
    backgroundColor: "{colors.sunken}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "0.375rem 0.875rem"
  button-danger-hover:
    backgroundColor: "{colors.danger}"
    textColor: "{colors.on-danger}"
  button-sm:
    typography: "{typography.meta}"
    rounded: "{rounded.control}"
    padding: "0.125rem 0.625rem"
  button-on-shell:
    backgroundColor: "{colors.shell-raised}"
    textColor: "{colors.shell-ink}"
    rounded: "{rounded.control}"
  input:
    backgroundColor: "{colors.field}"
    textColor: "{colors.ink}"
    typography: "{typography.base}"
    rounded: "{rounded.control}"
    padding: "0.5rem 0.75rem"
  input-disabled:
    backgroundColor: "{colors.sunken}"
    textColor: "{colors.ink-faint}"
  panel:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "{spacing.panel}"
  notice:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
    padding: "0.75rem 1rem"
  data-table:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
    rounded: "{rounded.control}"
  data-table-header:
    textColor: "{colors.ink-muted}"
    typography: "{typography.meta}"
    height: "2.25rem"
    padding: "0 0.75rem"
  data-table-row:
    height: "{spacing.row}"
    padding: "0.25rem 0.75rem"
  data-table-row-hover:
    backgroundColor: "{colors.sunken}"
  shell:
    backgroundColor: "{colors.shell}"
    textColor: "{colors.shell-ink}"
    height: "{spacing.bar}"
  shell-nav-link:
    textColor: "{colors.shell-muted}"
    padding: "0.5rem 0.75rem"
  shell-nav-link-current:
    textColor: "{colors.shell-ink}"
  theme-switch:
    backgroundColor: "{colors.sunken}"
    textColor: "{colors.ink-muted}"
    rounded: "{rounded.control}"
    padding: "0.125rem"
  theme-switch-pressed:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.ink}"
  heat-track:
    backgroundColor: "{colors.sunken}"
    height: "0.5rem"
    width: "6rem"
---

# Design System: Lemnos

## Overview

**Creative North Star: "Bronze at the Forge"**

Machines are Hephaistos's bronzes. One in use stays polished, one left idle grows a patina, and how hard it works is read as the heat the metal has run to. The whole system is that one reading: a deep verdigris bar across the top, a green-black or pale green-grey ground beneath it, polished bronze wherever something is running or waiting to be pressed, sea-green patina wherever something has been left alone.

There are two kinds of page. The console, behind sign-in, is dense and flat: a title, one line about the page, then full-width tables with 40px rows. The public site (the landing page and Features) is built from the console's own parts and only adds what frames them: a verdigris band under the bar holding one large line, and sections of plain text under hairline rules. Bordered panels hold a single form or a single group of settings and nothing else. There are no shadows, no cards used as layout, four type sizes and one corner radius. Greece is felt, not illustrated: the bronze and the verdigris, the island's name set small in Greek capitals beside the wordmark, and a text face drawn by a Greek designer.

Both modes are first-class. Every colour is one token holding a light value and a dark value, switched by the page's colour scheme, so nothing is styled twice. The bar and the heat scale are the same in both modes.

**Key Characteristics:**
- A verdigris bar on every page, identical in light and dark.
- A tinted ground: green-black in dark, cool pale green-grey in light, never neutral grey or cream.
- Bronze for what is in use, patina for what is idle, cinnabar for failure and nothing else.
- Load drawn as a heat strip clipped from one fixed gradient, so rows compare by eye.
- State shown as a mark and a word; the marks differ in shape as well as colour.
- Flat surfaces, hairline rules, 4px corners, 40px rows, four type sizes.

## Colors

A green-tinted neutral field carrying two metals, with one red held back for failure.

### Primary
- **Polished Bronze** (bronze): the one resting fill for primary actions (Start, Resume, Sign in, Add) and the mark for anything running, on or online. Also the underline under the current page in the bar, the text selection and the tick of a checkbox.
- **Worked Bronze** (bronze-hover): the primary button under the pointer. Darker in light mode, brighter in dark mode.
- **Bronze Ink** (bronze-text): bronze as text and line: links outside tables, the keyboard focus ring, the focused field border and the text caret. Darker than the fill on a light page so it stays readable.
- **Forge Black** (on-bronze): text on a bronze fill, in both modes.

### Secondary
- **Patina** (patina): idle, stopped, off. The hollow ring of a stopped machine, and the border and icon of a success notice.
- **Verdigris** (shell): the bar across the top of every page.
- **Raised Verdigris** (shell-raised): buttons and the theme switch where they sit on the bar.
- **Shell Ink** (shell-ink) and **Shell Muted** (shell-muted): text on the bar; muted for links at rest, ink for the current page and on hover. The same in both modes.

### Tertiary
- **Cinnabar** (danger): failure. The border and wash of a failure notice, the struck ring of something unreachable, and the fill of a Stop, Remove or Turn off button while it is under the pointer or focused.
- **Cinnabar Ink** (danger-text): cinnabar as text and icon: the word beside an unreachable mark, the icon of a failure notice, an inline error under a field.
- **White on Cinnabar** (on-danger): text on a cinnabar fill.
- **Straw Ink** (warning-text): the split disc of a paused machine, and the border and icon of a warning notice.

### Heat Scale
- **Heat 1 to 5** (heat-1 patina green, heat-2 bronze, heat-3 straw, heat-4 copper, heat-5 violet): stops of one gradient at 0%, 35%, 60%, 82% and 100%. They exist only inside load strips and are the same in both modes.

### Neutral
- **Ground** (ground): the page.
- **Surface** (surface): things lying on the page: panels, tables, plain notices, the pressed segment of the theme switch.
- **Sunken** (sunken): things pressed into the page: ordinary buttons, the empty track of a heat strip, a hovered table row, a disabled field.
- **Field** (field): the inside of a text field, lighter than what it sits on so it reads as empty.
- **Line** (line): hairlines: panel and table frames, rules between rows.
- **Strong Line** (line-strong): edges of controls and the rule under a table header; dark enough to find an empty field.
- **Ink** (ink), **Muted Ink** (ink-muted), **Faint Ink** (ink-faint): text; muted for the line under a title, labels, table headers and idle names; faint for hints, placeholders, the footer and the labels inside stacked rows.

### Named Rules
**The Two Metals Rule.** Bronze means in use, patina means idle. A running machine, a switched-on setting and an online node take the bronze disc; a stopped machine and a switched-off setting take the patina ring. Never swap them and never add a third "healthy" colour.

**The Failure-Only Red Rule.** At rest, cinnabar appears only where something has failed: a failure notice, an unreachable mark, an inline error. Stop, Remove and Turn off look like any other button until pointed at or focused, so a red notice is never one of many red things on the page.

**The Heat Stays In The Strip Rule.** The five heat colours are never used for text, fills, marks or decoration. Straw for "paused" comes from the warning token, not from the scale.

**The Tinted Ground Rule.** Every neutral leans green. No neutral grey, no cream, no pure black ground.

## Typography

**Display Font:** Commissioner (with ui-sans-serif, system-ui, sans-serif); weights 400, 500, 600, 700; Latin and Greek subsets, self-hosted.
**Body Font:** Commissioner, the same family.
**Label/Mono Font:** JetBrains Mono (with ui-monospace, monospace); weights 400 and 500, self-hosted.

**Character:** One humanist sans does everything, title to table cell, and carries the Greek capitals of the wordmark without a second face. The mono is a measuring instrument, not a voice.

### Hierarchy
- **Display** (700, 2.25rem to 4rem with the viewport, line-height 1.05, tracking -0.02em): the one line in the band at the top of the landing page. Nowhere else.
- **Title** (600, 1.5rem, line-height 2rem): the page title, one per page; on the landing page, the heading of a section.
- **Lead** (600, 1.125rem, line-height 1.5rem): section titles above a table or inside a panel; also the machine's name at the head of a stacked row (at its row weight) and the tagline under "Sign in" (400, muted).
- **Base** (400, 0.9375rem, line-height 1.5rem): everything else: body text, table cells, buttons and nav links (500), names of running things (500). Prose is capped at 65ch.
- **Meta** (500 for labels and table headers, 400 for hints, 0.8125rem, line-height 1.25rem): field labels, hints, table headers, small buttons, back links, the footer, the small facts line of a stacked machine. Sentence case, no tracking.
- **Figure** (JetBrains Mono 400, 0.8125rem, tabular numerals): IDs, addresses, percentages, sizes, the technical reason folded inside a failure notice.

### Named Rules
**The Four Sizes Rule.** Title, lead, base, meta. The default type scale is removed from the build, so a size outside these generates nothing. Hierarchy comes from weight and ink strength before it comes from size. The display size is the one exception, and it belongs to the landing page's first line only.

**The Digits Rule.** Mono is only for things compared or typed character by character: IDs, addresses, figures, usernames, tags, and a config file shown as it would be typed. Names, labels and prose are never mono.

**The Wordmark Rule.** The bar reads "Lemnos" (700, 1.125rem, tracking -0.01em) followed by ΛΗΜΝΟΣ (500, 0.8125rem, tracking 0.16em, shell-muted), on one baseline. Wide tracking and Greek capitals belong to this lockup only; the Greek is decorative and hidden from assistive technology.

## Layout

Three page widths, centred, with a 1rem gutter that becomes 1.5rem from 40rem up: 84rem for pages of tables, 44rem for settings, 26rem for a single form. The bar and footer share the 84rem width so their edges line up with the widest page. A page is a vertical stack with 2rem between blocks; a section title sits 0.75rem above its table; fields in a form are 1rem apart, a label 0.375rem above its input.

A page opens with its title and one muted line about it, with room for actions on the right. Detail pages put a small underlined back link above the title. Tables run the full width of the page; a form that belongs beside a table (adding an environment) takes a fixed 24rem column on the right from 64rem up and drops below the table under that.

Tables are dense: 36px header, 40px rows, 0.75rem cell padding, 1rem at the outer edges, power actions pushed to the right edge.

Responsive behaviour, as built:
- **Below 75rem (1200px)** a stacking table stops being a table. Its header is hidden from sight, each row becomes a block, and the blocks are laid out in as many columns as fit at a minimum of 21rem each: two across at 1024, one at 390. Inside a block the name leads at lead size, each other value carries its column name above it in faint meta text, strips take the full width so their scale still lines up block to block, empty cells disappear, buttons grow to a 40px minimum height, and the last action (Stop) sits apart at the right.
- **Machine tables** stack more tightly: name and state on one line, a line of small facts (ID, kind, environment), then the CPU and memory strips each with its label on the left.
- **Tables that do not stack** scroll sideways inside their frame.
- **Below 40rem (640px)** the nav drops to a second line of the bar, and the theme switch leaves the bar for the footer, where its buttons are 40px square instead of 28px.
- **From 90rem** heat tracks widen from 6rem to 9rem.

Signed-out visitors get the public site: the landing page at the root, Features, and the sign-in pages, under a bar with a Sign in button (Go to dashboard once signed in). Every console page redirects a visitor to the sign-in form and brings them back to the page they asked for.

The landing page, top to bottom: a verdigris band continuing the bar, with the display line, one muted sentence and two buttons; an example list of machines, drawn with the console's real table, lying half on the band; three statements side by side, each under a strong hairline; two split sections with words on the left and the thing they are about on the right (the legend of state marks, a config file); a closing line with the way to the source. Sections are 84rem wide like the console and separated by space, not by boxes.

## Elevation & Depth

Flat. Nothing casts a shadow. Depth is three tones of the same green: the ground, surfaces lying on it, and sunken things pressed into it, separated by 1px lines. The only shadow declaration in the system is an inset 1px line drawn around a heat track, used as a hairline.

### Named Rules
**The Flat Metal Rule.** No drop shadows, no glows, no blur, at rest or on hover. A thing is raised by being lighter than its ground and lowered by being darker; hover changes tone, never height.

## Shapes

One corner: 4px (0.25rem) on every button, field, panel, table frame, notice and the theme switch. The segments inside the theme switch are 1px tighter so they nest. Heat tracks are nearly square (1px). Frames are 1px lines; controls use the stronger line so an empty field can be found.

The only round or deliberately square shapes are the 10px state marks, where shape carries meaning: a solid disc, a hollow ring, a half-filled disc, a hollow square, a ring struck through.

### Named Rules
**The One Corner Rule.** Containers and controls share the 4px radius. No pills, no large rounded cards.

## Components

### Buttons
Quiet, pressed-in controls; only the one that moves things forward is bronze.
- **Shape:** 4px corners, 1px strong-line border, 0.375rem by 0.875rem padding, base size at weight 500.
- **Default:** sunken fill, ink text; hover darkens the fill to the line colour.
- **Primary:** bronze fill, forge-black text, no visible border; hover moves to worked bronze. One per form or row.
- **Danger:** identical to the default at rest. Under the pointer or keyboard focus it fills cinnabar with white text and a cinnabar border.
- **Small:** meta size, 0.125rem by 0.625rem padding, for table rows and the bar. In stacked rows it grows to a 40px minimum height.
- **On the bar:** raised-verdigris fill with shell ink; hover brightens it.
- **Disabled:** half opacity, no pointer.
- **Motion:** colours ease over 150ms; nothing changes size or position.

### State marks
A 10px mark and a word, always together.
- **Running / On / Online:** solid bronze disc, ink word.
- **Stopped / Off:** hollow patina ring, muted word.
- **Paused:** disc half-filled in straw.
- **Template:** hollow square in faint ink, muted word.
- **Unreachable / Offline / Unavailable:** cinnabar ring struck through diagonally, cinnabar word.

### Heat strip (signature)
How hard something is working. An 8px track, 6rem wide (9rem from 90rem), sunken fill with a hairline edge, followed by its figure in muted mono. The fill is one fixed gradient spanning the whole track (the five heat stops) and revealed only up to the value, so a longer strip is also a hotter one and any two rows compare by eye. A stopped machine shows no CPU strip and an empty memory track beside its allocated size. On load each strip draws in once from the left over 0.8s on an ease-out-expo curve; with reduced motion it is simply there. The track is hidden from assistive technology; the figure is the accessible value.

### Tables
- **Frame:** surface fill, 1px line, 4px corners.
- **Header:** 36px, meta size at weight 500, muted ink, a strong-line rule beneath.
- **Rows:** 40px, hairline between rows, sunken on hover.
- **Name cell:** weight 500; in an idle row it drops to 400 and muted ink.
- **Links in tables:** ink, not bronze, with a permanent strong-line underline that takes the text colour on hover. A table full of names must not read as a table full of accents.
- **Empty table:** one 96px cell with a centred muted sentence saying what to do next.
- **Narrow screens:** see Layout.

### Panels
- **Corner Style:** 4px.
- **Background:** surface.
- **Shadow Strategy:** none; see Elevation & Depth.
- **Border:** 1px line.
- **Internal Padding:** 1.25rem, 1.5rem from 40rem up; 1rem between children.
- One form or one group of settings per panel. Never nested.

### Inputs / Fields
- **Style:** field fill, 1px strong-line border, 4px corners, 0.5rem by 0.75rem padding. Label above in muted meta at weight 500; optional hint below in faint meta.
- **Hover:** border moves to faint ink.
- **Focus:** border and a 2px ring in bronze ink, flush to the edge.
- **Disabled:** sunken fill, faint text.
- **Error:** an inline message in cinnabar ink below the field.
- **Checkbox:** native, 16px, ticked in bronze, label beside it.

### Notices
A bordered strip that says what just happened: a drawn 18px line icon, then the message.
- **Plain:** surface fill, line border, muted icon (circled i).
- **Success:** patina border and icon (circled tick) over a wash of 10% patina in the ground.
- **Warning:** straw border and icon (triangle) over a 10% straw wash.
- **Failure:** cinnabar border, cinnabar-ink icon (triangle) over a 10% cinnabar wash. It says what the failure means for the reader, may end with a bronze link to where to look, and folds the technical reason away under "Technical details" in mono. An unreachable environment is one of these above the table, never a broken page.

### Facts
A two-column list for what is known about one thing (a profile, a person's organisations): the name of the fact in muted ink, its value beside it, hairlines between rows. Used inside a panel instead of a table when there is one of each fact.

### Steps
A numbered list under hairlines, the number in bronze-ink mono at the left. On the landing page it explains; on the welcome page each step also carries a state mark (Done, To do) and a button, and only the step to do next gets the bronze one.

### Navigation
- **Bar:** verdigris, at least 56px tall, content held to 84rem. Wordmark, then the nav, then at the right the organisation being looked at, the theme switch, the account name and Sign out. Admins get Machines, Environments, Organisations and Users in the nav; everyone else Machines and Organisations. On a narrow bar the nav scrolls sideways.
- **Scope menu:** whose machines are shown. A small raised-verdigris button with a chevron that opens a list on the surface below it, with a 2px bronze line beside the current choice. A `<details>` element, so it opens without a script; with only one choice it is plain muted text.
- **Tabs:** the pages of one section (Settings: Account, Security), in the 44rem column above the page title. Muted links on a 2px line; the current one is ink on a bronze line.
- **Nav links:** base size at weight 500, shell-muted; shell ink on hover. The current page is shell ink with a 2px bronze line along the bottom edge of the bar.
- **Focus on the bar:** the ring is shell ink instead of bronze ink.
- **Theme switch:** three icon buttons in one segmented control: follow this device, light, dark. The pressed one is a lighter segment. 28px segments on the bar, 40px in the footer on narrow screens.
- **Footer:** one line of faint meta text, the name and tagline.
- **Links in text:** bronze ink at weight 500, underline appears on hover. Back links are muted meta text with a permanent underline.

## Do's and Don'ts

### Do:
- **Do** use the bronze fill for the single action that moves things forward, and the bronze disc for anything running, on or online.
- **Do** show state as a mark plus a word, and give a new state its own shape, not just its own colour.
- **Do** show load with the heat strip and put the exact figure beside it in mono.
- **Do** make destructive buttons look ordinary at rest and turn cinnabar only on hover and focus.
- **Do** define a colour once as a light and dark pair and let the colour scheme pick; the bar and heat scale stay the same in both modes.
- **Do** keep to the four sizes, the 4px corner and the 40px row.
- **Do** give every stacking table cell its column name, so the row still explains itself as a block below 75rem.
- **Do** report a failure as a notice in plain words with the technical reason folded beneath it.
- **Do** keep targets at least 40px tall where rows stack and where the theme switch sits in the footer.

### Don't:
- **Don't** put cinnabar on anything at rest that has not failed.
- **Don't** use the heat colours outside a load strip.
- **Don't** colour table links bronze; in tables they are ink with an underline.
- **Don't** add drop shadows, glows or blur.
- **Don't** nest panels, or build a page out of panels where a table or plain text would do.
- **Don't** use a neutral grey, cream or pure black ground.
- **Don't** set names, labels or prose in mono, or add a fifth type size.
- **Don't** animate anything except the heat strips drawing in once on load and 150ms colour changes on hover and focus.
- **Don't** use a green dot for "running"; green here means idle.
- **Don't** reuse the tracked Greek capitals of the wordmark as a label style elsewhere.
- **Don't** use the display size, or the verdigris band, inside the console.
- **Don't** show anything on the public site that is not true of the product; example data is labelled as such.
