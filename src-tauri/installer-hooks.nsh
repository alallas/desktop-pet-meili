; 《潮间带》NSIS 安装钩子（工程规范 §9.1 / 发版 checklist）
; 用途：sqlite3.dll 是动态链接的官方预编译库（坑 25），Windows 只在 exe 同级
; 目录找 DLL，而 tauri resources 落在 resources/ 子目录——安装后把它拷到 exe 旁边。
; 钩子名见 tauri v2 schema：NSIS_HOOK_POSTINSTALL / NSIS_HOOK_PREUNINSTALL

!macro NSIS_HOOK_POSTINSTALL
  ; 安装完成后：resources\sqlite3.dll → 安装根目录（exe 旁边）
  CopyFiles /SILENT "$INSTDIR\resources\sqlite3.dll" "$INSTDIR\sqlite3.dll"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 卸载前连带删除（卸载器默认只删自己登记的文件）
  Delete "$INSTDIR\sqlite3.dll"
!macroend
