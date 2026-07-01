@echo off
chcp 65001 >nul
title 词云工具 - 打包中

echo ============================================
echo   微信聊天词云生成器 - 打包脚本
echo ============================================
echo.

:: 切换到脚本所在目录
cd /d "%~dp0"

echo [1/3] 检查 PyInstaller...
pip show pyinstaller >nul 2>&1
if %errorlevel% neq 0 (
    echo   → 未安装，正在安装...
    pip install pyinstaller -i https://pypi.tuna.tsinghua.edu.cn/simple
) else (
    echo   → 已安装，跳过
)
echo.

echo [2/3] 开始打包为单个 exe...
pyinstaller --onefile ^
    --name "词云生成器" ^
    --add-data "utils;utils" ^
    --hidden-import jieba.analyse ^
    --clean ^
    --console ^
    generate.py

echo.
echo [3/3] 清理临时文件...
rmdir /s /q build >nul 2>&1
del *.spec >nul 2>&1

echo.
echo ============================================
echo   ✅ 打包完成！
echo.
echo   输出文件: dist\词云生成器.exe
echo.
echo   使用方式：
echo     1. 将 WeFlow 导出的 JSON 文件
echo        拖放到 词云生成器.exe 图标上
echo     2. 或在命令行运行：
echo        词云生成器.exe 聊天记录.json
echo ============================================

pause
