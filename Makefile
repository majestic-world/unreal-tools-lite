.PHONY: icon build dev

icon:
	pnpm tauri icon .\src-tauri\icons\app-icon.png

build:
	pwsh -NoProfile -ExecutionPolicy Bypass -File .\scripts\build.ps1

dev: 
	pnpm tauri dev

	