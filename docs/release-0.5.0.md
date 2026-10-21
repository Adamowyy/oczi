Oczi for Windows 0.5.0.

**The PC pill became a toolbox.** Three things now live behind one pill, none of them needing a key.

### Health

- One click checks the machine and answers with a score and a list: the load right now, three days of the event log, empty autostart entries, disks, a waiting restart. Every line says what it means instead of leaving a code to look up.
- **Deeper** reads a week of crash reports: the module that took a program down, what the bugcheck code is about, and a paragraph from the model saying what matters and what to do.
- A question in the chat reaches the last scan too, so "why does it stutter" is answered from the machine rather than from memory.

### Debloat

- What Windows brought with it, split into **safe** and **careful**, with tick boxes. Only what you tick is removed, and the result is read back from Windows instead of being taken from the script that did it.
- Empty autostart entries can be switched off the way Task Manager does it — and switched back on there.

### Freeze watch

- A three-second freeze leaves nothing in any log, so the watch samples the machine once a second for fifteen or thirty minutes and writes down every moment it stopped, with the process that was on the processor then — or the driver time, when a driver is what stalled it.
- You keep using the computer normally. It ends by itself and nothing keeps running afterwards; the result stays on screen until you acknowledge it.

**Oczi-Windows-0.5.0-setup.exe** is the ordinary Oczi, about 2 MB.
**Oczi-Windows-0.5.0-voice-setup.exe** is the same Oczi with dictation, a few hundred MB because the speech model is inside.

Both installers are unsigned, so Windows shows its SmartScreen warning on the first run.
