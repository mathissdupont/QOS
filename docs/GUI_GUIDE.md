# QOS Desktop Environment - GUI Guide

## 🎨 Overview

QOS now includes a **Windows-style Desktop Environment** with multiple applications, taskbar, and desktop icons!

## 🚀 Getting Started

### Starting the Desktop

```bash
# Start desktop with demo windows
desktop

# Or launch individual apps
calc        # Calculator
notepad     # Text editor
explorer    # File browser
taskmgr     # Task manager
sysinfo     # System information
```

## 📱 Available Applications

### 1. Calculator (`calc`)
A functional calculator with basic arithmetic operations.

Features:
- Addition, subtraction, multiplication, division
- Decimal support
- Clear display
- Windows-style button layout

### 2. Notepad (`notepad`)
A simple text editor for creating and viewing text files.

Features:
- Multi-line text editing
- File operations (open, save)
- Character and line count
- Menu bar (File, Edit, Format, View, Help)

### 3. File Explorer (`explorer`)
Browse and manage files across the virtual file system.

Features:
- Directory navigation
- File size and type display
- Operations: Open, Copy, Delete
- Create new folders
- Refresh view

### 4. Task Manager (`taskmgr`)
Monitor system processes and resources.

Features:
- Process list with PID and status
- Exit code display
- System uptime
- Real-time clock
- End task functionality

### 5. System Information (`sysinfo`)
View comprehensive system information.

Displays:
- QOS version and architecture
- System time and uptime
- Memory status
- Feature list
- Kernel information

## 🎯 Desktop Features

### Window Management
- **Multiple Windows**: Run several apps simultaneously
- **Focus Management**: Click to focus different windows
- **Window Controls**: Minimize (_), Maximize (□), Close (X)
- **Overlapping**: Windows stack on top of each other

### Taskbar
- **Start Button**: Quick access menu (QOS button)
- **Window Buttons**: Switch between open windows
- **System Tray**: Clock display (expandable)
- **Active Indicator**: Highlighted for focused window

### Desktop Icons
- 💻 **Computer**: System resources
- 📁 **Files**: Quick file access
- ⌨ **Terminal**: Shell access
- ⚙ **Settings**: System configuration

## 🖱️ Mouse Support

The desktop supports PS/2 mouse with scroll wheel:
- **Left Click**: Focus windows, click buttons
- **Right Click**: Context menu (planned)
- **Scroll**: Navigate content (planned)

## ⌨️ Keyboard Shortcuts

```
Tab         - Switch between windows
Alt+F4      - Close focused window (planned)
Win+D       - Show desktop (planned)
Ctrl+Alt+T  - Open terminal (planned)
```

## 📊 Technical Details

### Display Mode
- **Resolution**: 80x25 (VGA text mode)
- **Colors**: 16-color palette
- **Characters**: Extended ASCII with box-drawing

### Architecture
```
Desktop Manager
  ├── Window System
  │   ├── Window rendering
  │   ├── Focus management
  │   └── Z-order handling
  ├── Taskbar
  │   ├── Start menu
  │   ├── Window buttons
  │   └── System tray
  └── Icons
      └── Desktop shortcuts
```

## 🛠️ Creating Custom Windows

You can create custom windows from the shell:

```bash
# Create a basic window
window "My Window"

# The window ID is returned for further operations
```

## 🎨 Color Scheme

- **Wallpaper**: Cyan with light pattern (░)
- **Taskbar**: Dark gray background
- **Window Borders**: Blue (focused), Gray (unfocused)
- **Title Bar**: Blue with white text
- **Window Content**: White background, black text

## 🔄 Future Enhancements

Planned features:
- [ ] Drag and drop windows
- [ ] Window resizing
- [ ] Context menus
- [ ] More keyboard shortcuts
- [ ] Application launcher
- [ ] Settings panel
- [ ] Paint application
- [ ] Music player
- [ ] Network browser
- [ ] VESA graphics mode (higher resolution)

## 📝 Example Session

```bash
# Start QOS and enter shell
QaOS:/ $ desktop
Starting Desktop Environment...
Desktop initialized with welcome windows

# Launch calculator in new terminal
QaOS:/ $ calc
Calculator launched

# Open file browser
QaOS:/ $ explorer
File Explorer launched

# Check system info
QaOS:/ $ sysinfo
System Information launched

# Get help
QaOS:/ $ help gui
[Shows GUI help with all commands]
```

## 🐛 Known Limitations

1. **Text Mode Only**: Currently runs in VGA text mode (80x25)
   - Future: VESA framebuffer for higher resolutions
2. **No Mouse Drag**: Mouse clicking works, but dragging windows is not yet implemented
3. **Static Icons**: Desktop icons are decorative (click handlers coming soon)

## 🎯 Best Practices

1. **Start Desktop First**: Run `desktop` command to initialize the environment
2. **Launch Apps**: Use individual commands (`calc`, `notepad`, etc.) to open apps
3. **Use Help**: Type `help gui` for quick reference
4. **Check Status**: `taskmgr` shows running processes

## 💡 Tips

- **Multiple Apps**: You can run multiple instances of the same app
- **Window Overlapping**: Last opened window gets focus
- **Taskbar**: Shows all non-minimized windows
- **Performance**: Desktop rendering is optimized for real-time updates

## 🌟 Showcase

Try this demo sequence:
```bash
desktop      # Start with welcome screens
calc         # Add calculator
explorer     # Add file browser
taskmgr      # Monitor processes
notepad      # Take notes
sysinfo      # View system info
```

This creates a full desktop experience with 6+ windows!

---

**QOS Desktop Environment v1.0**  
*Windows-style GUI for Quantum Operating System*
