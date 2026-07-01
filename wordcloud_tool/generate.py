#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
微信聊天记录词云生成工具
=========================
读取 WeFlow 导出的聊天记录 JSON/TXT，进行中文分词并生成词云。

用法:
    python generate.py <导出的JSON文件或目录>
    python generate.py ./exported_chat.json
    python generate.py ./export_data/          # 扫描目录下所有JSON

输出:
    output/wordcloud.png        - 词云图片
    output/chart_freq.png       - 高频词分布图
    output/report.html          - 词云报告（交互式HTML）
"""

import argparse
import json
import os
import re
import sys
from collections import Counter
from pathlib import Path

import jieba
import jieba.analyse
import matplotlib
import matplotlib.pyplot as plt
import numpy as np
from wordcloud import WordCloud

matplotlib.use("Agg")  # 无头模式（不弹窗口）

# ──────────────────────────────────────────────
#  配置
# ──────────────────────────────────────────────

def get_resource_path(relative_path: str) -> Path:
    """获取资源文件的路径（兼容 PyInstaller 打包）"""
    if hasattr(sys, "_MEIPASS"):
        # PyInstaller 打包后，资源文件在 _MEIPASS 临时目录下
        return Path(sys._MEIPASS) / relative_path
    return Path(__file__).parent / relative_path


UTILS_DIR = get_resource_path("utils")
STOPWORDS_PATH = UTILS_DIR / "stopwords.txt"
USER_DICT_PATH = UTILS_DIR / "user_dict.txt"

OUTPUT_DIR = Path("output")
WORDCLOUD_WIDTH = 1600
WORDCLOUD_HEIGHT = 1000
MAX_WORDS = 200
MIN_WORD_LENGTH = 2        # 最短词长度（中文2字起步）
MIN_FREQUENCY = 2           # 最低词频
TOP_WORDS_DISPLAY = 80      # HTML报告展示top数

# Windows 中文字体备选
FONT_CANDIDATES = [
    "C:/Windows/Fonts/msyh.ttc",        # 微软雅黑
    "C:/Windows/Fonts/msyhbd.ttc",      # 微软雅黑加粗
    "C:/Windows/Fonts/simhei.ttf",      # 黑体
    "C:/Windows/Fonts/SimHei.ttf",
    "C:/Windows/Fonts/simsun.ttc",      # 宋体
    "/System/Library/Fonts/PingFang.ttc",   # macOS
    "/System/Library/Fonts/STHeiti Light.ttc",
    "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",  # Linux
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
]

# ──────────────────────────────────────────────
#  工具函数
# ──────────────────────────────────────────────

def find_chinese_font() -> str:
    """寻找系统可用的中文字体"""
    for fp in FONT_CANDIDATES:
        if os.path.exists(fp):
            return fp
    # fallback: 让 wordcloud 自己做字体检测
    return None


def load_stopwords(path: Path) -> set:
    """加载停用词表"""
    stopwords = set()
    if not path.exists():
        print(f"[警告] 停用词表不存在: {path}")
        return stopwords
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            word = line.strip()
            if word and not word.startswith("#"):
                stopwords.add(word)
    print(f"[信息] 停用词表加载: {len(stopwords)} 个")
    return stopwords


def load_user_dict(path: Path):
    """加载自定义词典（结巴分词用）"""
    if not path.exists():
        print(f"[警告] 自定义词典不存在: {path}")
        return
    jieba.load_userdict(str(path))
    print(f"[信息] 自定义词典加载完成")


def clean_text(text: str) -> str:
    """清理聊天文本：去除非文本噪声"""
    if not text:
        return ""
    # 微信表情符: [呲牙] [流泪] [强] 等
    text = re.sub(r"\[.+?\]", "", text)
    # URL
    text = re.sub(r"https?://\S+", "", text)
    # 纯数字（长度≥4 的数字可能是订单号、手机号等）
    text = re.sub(r"\b\d{4,}\b", "", text)
    # email
    text = re.sub(r"\S+@\S+", "", text)
    # 重复标点/符号
    text = re.sub(r"([，。！？、；：])\1+", r"\1", text)
    # 空白字符归一化
    text = re.sub(r"\s+", "", text)  # 中文分词不需要空格
    return text.strip()


def is_text_message(msg: dict) -> bool:
    """判断是否为纯文本消息"""
    local_type = msg.get("localType", msg.get("local_type", 0))
    msg_type = msg.get("type", "")
    # WeFlow JSON: localType=1 或 type="文本"
    return local_type == 1 or msg_type == "文本"


def extract_message_content(msg: dict) -> str:
    """从消息中提取文本内容"""
    content = msg.get("content", "")
    if not content:
        return ""
    # 过滤非文本系统消息
    non_text_markers = [
        "[图片]", "[语音]", "[视频]", "[表情]", "[链接]",
        "[文件]", "[名片]", "[位置]", "[红包]", "[转账]",
        "[小程序]", "[音乐]", "[通话]",
    ]
    for marker in non_text_markers:
        if content.startswith(marker):
            return ""
    return content


def guess_encoding(filepath: str) -> str:
    """猜测文件编码"""
    encodings = ["utf-8", "utf-8-sig", "gbk", "gb18030", "latin-1"]
    for enc in encodings:
        try:
            with open(filepath, "r", encoding=enc) as f:
                f.read(1024)
            return enc
        except (UnicodeDecodeError, UnicodeError):
            continue
    return "utf-8"


# ──────────────────────────────────────────────
#  数据解析
# ──────────────────────────────────────────────

def parse_weflow_json(filepath: str) -> list:
    """解析 WeFlow 导出的 JSON 文件，返回消息文本列表"""
    encoding = guess_encoding(filepath)
    with open(filepath, "r", encoding=encoding) as f:
        data = json.load(f)

    session_info = data.get("session", {})
    session_name = session_info.get("displayName", session_info.get("nickname", Path(filepath).stem))
    session_type = session_info.get("type", "未知")
    message_count = session_info.get("messageCount", 0)

    print(f"  ├ 会话: {session_name} ({session_type})")
    print(f"  ├ 消息总量: {message_count}")

    messages = data.get("messages", [])
    if not messages:
        print(f"  └ [警告] 无消息数据")
        return []

    # 提取文本类消息
    texts = []
    for msg in messages:
        if not is_text_message(msg):
            continue
        content = extract_message_content(msg)
        if content:
            texts.append(content)

    print(f"  └ 文本消息: {len(texts)} 条")
    return texts


def parse_weflow_txt(filepath: str) -> list:
    """解析 TXT 导出格式（按行提取文本）"""
    encoding = guess_encoding(filepath)
    texts = []

    with open(filepath, "r", encoding=encoding) as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            # 跳过表头/分割线
            if any(kw in line for kw in ["序号", "时间", "内容", "---", "==="]):
                continue
            # TXT 通常有列分隔符，取最后一列作为内容
            text = line.split("\t")[-1] if "\t" in line else line
            texts.append(text)

    print(f"  └ TXT 文本行: {len(texts)} 条")
    return texts


def load_data(input_path: str) -> list:
    """加载数据入口：支持文件或目录"""
    path = Path(input_path)

    if path.is_dir():
        # 扫描目录下所有 JSON / TXT
        all_texts = []
        json_files = sorted(path.rglob("*.json"))
        txt_files = sorted(path.rglob("*.txt"))
        if not json_files and not txt_files:
            print(f"[错误] 目录下没有 JSON 或 TXT 文件: {input_path}")
            sys.exit(1)
        for jf in json_files:
            print(f"\n[读取] {jf.name}")
            texts = parse_weflow_json(str(jf))
            all_texts.extend(texts)
        for tf in txt_files:
            print(f"\n[读取] {tf.name}")
            texts = parse_weflow_txt(str(tf))
            all_texts.extend(texts)
        return all_texts

    elif path.is_file():
        ext = path.suffix.lower()
        if ext == ".json":
            print(f"\n[读取] {path.name}")
            return parse_weflow_json(input_path)
        elif ext == ".txt":
            print(f"\n[读取] {path.name}")
            return parse_weflow_txt(input_path)
        else:
            print(f"[错误] 不支持的文件格式: {ext}（支持 .json .txt）")
            sys.exit(1)
    else:
        print(f"[错误] 路径不存在: {input_path}")
        sys.exit(1)


# ──────────────────────────────────────────────
#  分词与词频统计
# ──────────────────────────────────────────────

def segment_and_count(texts: list, stopwords: set, min_len: int = MIN_WORD_LENGTH) -> Counter:
    """
    对文本列表进行结巴分词，返回词频 Counter。
    策略:
      - jieba.lcut 精确模式
      - 过滤停用词
      - 过滤单字
      - 过滤纯数字/标点
    """
    word_freq = Counter()

    for i, text in enumerate(texts):
        text = clean_text(text)
        if not text:
            continue
        words = jieba.lcut(text, cut_all=False, HMM=True)
        for w in words:
            w = w.strip()
            if len(w) < min_len:
                continue
            if w in stopwords:
                continue
            if re.match(r"^[\d\W_]+$", w):
                continue
            word_freq[w] += 1

        # 进度提示（每 2000 条）
        if (i + 1) % 2000 == 0:
            print(f"  └ 分词进度: {i+1}/{len(texts)}")

    return word_freq


def extract_key_phrases(texts: list, top_k: int = 100) -> list:
    """
    使用 TextRank 提取关键词/短语，
    返回 [(phrase, weight), ...]
    """
    full_text = " ".join(texts)  # jieba.textrank 需要空格分隔的文本
    if not full_text.strip():
        return []
    try:
        keywords = jieba.analyse.textrank(
            full_text, topK=top_k, withWeight=True, allowPOS=("ns", "n", "vn", "v", "a", "ns", "nr")
        )
        return keywords
    except Exception as e:
        print(f"  [警告] TextRank 提取失败: {e}")
        return []


def merge_word_frequencies(
    word_freq: Counter,
    phrases: list,
    min_freq: int = MIN_FREQUENCY,
    max_words: int = MAX_WORDS,
) -> dict:
    """
    合并分词词频 + TextRank 短语，返回 {word: frequency} 供词云使用。
    短语权重放大 1.5 倍，使其更容易在词云中显现。
    """
    result = {}

    # 过滤低频词
    for word, count in word_freq.items():
        if count >= min_freq:
            result[word] = count

    # 加入 TextRank 短语（权重归一化后 + 放大）
    if phrases:
        max_weight = max(w for _, w in phrases)
        for phrase, weight in phrases:
            if phrase not in result and len(phrase) >= min_len_for_phrase(phrase):
                # 将权重映射到可感知的频率值
                boost = 1.5
                scaled = max(weight / max_weight * 50 * boost, min_freq)
                if scaled >= min_freq:
                    result[phrase] = round(scaled)

    # 按频率降序排序，取 top
    sorted_items = sorted(result.items(), key=lambda x: -x[1])
    result = dict(sorted_items[:max_words])

    return result


def min_len_for_phrase(phrase: str) -> int:
    """短语的最小长度要求：有中文至少2字，纯英文至少3字母"""
    chinese_chars = re.findall(r"[一-鿿]", phrase)
    if chinese_chars:
        return 2
    return 3


# ──────────────────────────────────────────────
#  词云生成
# ──────────────────────────────────────────────

def generate_wordcloud(word_freq: dict, output_dir: Path):
    """生成词云图片"""
    font_path = find_chinese_font()
    if font_path:
        print(f"[信息] 使用字体: {font_path}")

    wc = WordCloud(
        font_path=font_path,
        width=WORDCLOUD_WIDTH,
        height=WORDCLOUD_HEIGHT,
        max_words=MAX_WORDS,
        background_color="white",
        colormap="viridis",
        max_font_size=180,
        min_font_size=14,
        random_state=42,
        prefer_horizontal=0.65,
        relative_scaling=0.5,
        margin=8,
    )

    wc.generate_from_frequencies(word_freq)
    output_path = output_dir / "wordcloud.png"
    wc.to_file(str(output_path))
    print(f"[输出] 词云图片: {output_path}")
    return str(output_path)


# ──────────────────────────────────────────────
#  统计图表
# ──────────────────────────────────────────────

def generate_frequency_chart(top_words: list, output_dir: Path):
    """生成高频词分布柱状图"""
    if not top_words:
        return None

    words = [w for w, _ in top_words[:30]][::-1]
    counts = [c for _, c in top_words[:30]][::-1]

    plt.rcParams["font.sans-serif"] = ["Microsoft YaHei", "SimHei", "DejaVu Sans"]
    plt.rcParams["axes.unicode_minus"] = False

    fig, ax = plt.subplots(figsize=(12, 10), facecolor="#1a1a2e")
    ax.set_facecolor("#1a1a2e")

    colors = plt.cm.viridis(np.linspace(0.2, 0.9, len(words)))
    bars = ax.barh(range(len(words)), counts, color=colors, edgecolor="none", height=0.7)

    ax.set_yticks(range(len(words)))
    ax.set_yticklabels(words, fontsize=11, color="#e0e0e0")
    ax.tick_params(axis="x", colors="#e0e0e0", labelsize=9)
    ax.set_xlabel("出现次数", fontsize=12, color="#e0e0e0")
    ax.set_title("高频词分布 Top 30", fontsize=16, color="#ffffff", pad=15)

    for bar, count in zip(bars, counts):
        ax.text(
            bar.get_width() + max(counts) * 0.005,
            bar.get_y() + bar.get_height() / 2,
            str(count),
            va="center",
            fontsize=9,
            color="#e0e0e0",
        )

    ax.spines["top"].set_visible(False)
    ax.spines["right"].set_visible(False)
    ax.spines["left"].set_color("#333")
    ax.spines["bottom"].set_color("#333")
    ax.set_xlim(0, max(counts) * 1.12)

    plt.tight_layout()
    output_path = output_dir / "chart_freq.png"
    plt.savefig(output_path, dpi=150, bbox_inches="tight", facecolor=fig.get_facecolor())
    plt.close()
    print(f"[输出] 频率分布图: {output_path}")
    return str(output_path)


# ──────────────────────────────────────────────
#  HTML 报告
# ──────────────────────────────────────────────

def generate_html_report(
    top_words: list,
    wordcloud_path: str,
    chart_path: str,
    stats: dict,
    output_dir: Path,
):
    """生成交互式 HTML 词云报告"""
    # 构建 top 词表格行
    table_rows = ""
    for i, (word, count) in enumerate(top_words[:TOP_WORDS_DISPLAY], 1):
        percentage = count / stats["total_segments"] * 100 if stats["total_segments"] > 0 else 0
        bar_width = count / top_words[0][1] * 100 if top_words else 0
        table_rows += f"""
        <tr>
            <td class="rank">{i}</td>
            <td class="word">{word}</td>
            <td class="count">{count}</td>
            <td class="pct">{percentage:.2f}%</td>
            <td class="bar-cell"><div class="bar" style="width:{bar_width:.1f}%"></div></td>
        </tr>"""

    chart_section = ""
    if chart_path:
        chart_section = f"""
    <div class="chart-section">
        <img src="{Path(chart_path).name}" alt="Frequency Chart">
    </div>"""

    html = f"""<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>微信聊天词云报告</title>
<style>
* {{ margin: 0; padding: 0; box-sizing: border-box; }}
body {{
    font-family: -apple-system, "Microsoft YaHei", "PingFang SC", "Helvetica Neue", sans-serif;
    background: #0f0f1a;
    color: #e0e0e0;
    min-height: 100vh;
}}
.container {{ max-width: 1100px; margin: 0 auto; padding: 20px; }}
header {{
    text-align: center; padding: 40px 0 30px;
    background: linear-gradient(135deg, #1a1a2e 0%, #16213e 50%, #0f3460 100%);
    border-radius: 16px; margin-bottom: 30px;
}}
header h1 {{ font-size: 28px; font-weight: 700; color: #fff; letter-spacing: 2px; }}
header p {{ color: #8899aa; margin-top: 8px; font-size: 14px; }}
.stats-grid {{
    display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
    gap: 16px; margin-bottom: 30px;
}}
.stat-card {{
    background: #1a1a2e; border-radius: 12px; padding: 20px; text-align: center;
    border: 1px solid #2a2a3e; transition: transform 0.2s;
}}
.stat-card:hover {{ transform: translateY(-3px); }}
.stat-card .number {{ font-size: 32px; font-weight: 700; color: #4fc3f7; }}
.stat-card .label {{ font-size: 13px; color: #8899aa; margin-top: 6px; }}
.wordcloud-section {{ margin-bottom: 30px; }}
.wordcloud-section img {{
    width: 100%; border-radius: 12px;
    box-shadow: 0 8px 32px rgba(0,0,0,0.4);
    display: block;
}}
.chart-section {{ margin-bottom: 30px; }}
.chart-section img {{
    width: 100%; border-radius: 12px;
    box-shadow: 0 8px 32px rgba(0,0,0,0.4);
    display: block;
}}
.table-section {{ background: #1a1a2e; border-radius: 12px; padding: 20px; border: 1px solid #2a2a3e; }}
.table-section h2 {{ font-size: 18px; margin-bottom: 16px; color: #fff; }}
table {{ width: 100%; border-collapse: collapse; }}
th {{
    text-align: left; padding: 10px 12px; font-size: 12px; text-transform: uppercase;
    color: #8899aa; border-bottom: 2px solid #2a2a3e; font-weight: 600;
}}
td {{
    padding: 10px 12px; border-bottom: 1px solid #222;
    font-size: 14px; vertical-align: middle;
}}
tr:hover td {{ background: rgba(255,255,255,0.03); }}
.rank {{ color: #8899aa; width: 40px; text-align: center; }}
.word {{ font-weight: 500; color: #e0e0e0; }}
.count {{ color: #4fc3f7; font-weight: 600; text-align: right; }}
.pct {{ color: #8899aa; text-align: right; font-size: 13px; }}
.bar-cell {{ width: 30%; }}
.bar {{
    height: 8px; background: linear-gradient(90deg, #4fc3f7, #1a73e8);
    border-radius: 4px; min-width: 2px; transition: width 0.3s;
}}
footer {{
    text-align: center; padding: 30px 0; color: #556; font-size: 13px;
}}
@media (max-width: 600px) {{
    .stats-grid {{ grid-template-columns: repeat(2, 1fr); }}
    .bar-cell {{ display: none; }}
}}
</style>
</head>
<body>
<div class="container">
    <header>
        <h1>☁ 微信聊天词云报告</h1>
        <p>基于 {stats['message_count']} 条消息 · {stats['total_segments']} 个分词 · {stats['unique_words']} 个不同词语</p>
    </header>

    <div class="stats-grid">
        <div class="stat-card"><div class="number">{stats['message_count']:,}</div><div class="label">消息总数</div></div>
        <div class="stat-card"><div class="number">{stats['total_segments']:,}</div><div class="label">分词总数</div></div>
        <div class="stat-card"><div class="number">{stats['unique_words']:,}</div><div class="label">不同词语</div></div>
        <div class="stat-card"><div class="number">{stats['top1_word']}</div><div class="label">最热词</div></div>
    </div>

    <div class="wordcloud-section">
        <img src="{Path(wordcloud_path).name}" alt="WordCloud">
    </div>

    {chart_section}

    <div class="table-section">
        <h2>🏆 高频词排行 Top {min(TOP_WORDS_DISPLAY, len(top_words))}</h2>
        <table>
            <thead>
                <tr><th>#</th><th>词语</th><th>次数</th><th>占比</th><th></th></tr>
            </thead>
            <tbody>
                {table_rows}
            </tbody>
        </table>
    </div>

    <footer>Generated by WeChat WordCloud Tool</footer>
</div>
</body>
</html>"""

    output_path = output_dir / "report.html"
    with open(output_path, "w", encoding="utf-8") as f:
        f.write(html)
    print(f"[输出] HTML 报告: {output_path}")
    return str(output_path)


# ──────────────────────────────────────────────
#  EXE 拖放支持
# ──────────────────────────────────────────────

def exe_drop_mode():
    """
    PyInstaller 单文件 exe 拖放模式：
    如果 exe 被直接双击运行（无参数），弹出提示引导用户拖放文件。
    """
    if getattr(sys, "frozen", False) and len(sys.argv) == 1:
        import tkinter as tk
        from tkinter import filedialog, messagebox

        root = tk.Tk()
        root.withdraw()
        messagebox.showinfo(
            "使用说明",
            "请将 WeFlow 导出的 JSON 聊天记录文件\n"
            "拖放到本 exe 图标上运行。\n\n"
            "或者：\n"
            "1. 将 JSON 文件拖到本窗口\n"
            "2. 或点击确定选择文件",
        )
        file_path = filedialog.askopenfilename(
            title="选择 WeFlow 导出的 JSON 聊天记录",
            filetypes=[("JSON 文件", "*.json"), ("文本文件", "*.txt"), ("所有文件", "*.*")],
        )
        if file_path:
            root.destroy()
            sys.argv = [sys.argv[0], file_path]
            return True
        root.destroy()
        return False
    return True


# ──────────────────────────────────────────────
#  主流程
# ──────────────────────────────────────────────

def main():
    if not exe_drop_mode():
        return

    parser = argparse.ArgumentParser(
        description="微信聊天记录词云生成工具 - 读取 WeFlow 导出的聊天记录，生成词云"
    )
    parser.add_argument(
        "input",
        help="WeFlow 导出的 JSON 文件路径，或包含 JSON 文件的目录路径",
    )
    parser.add_argument(
        "-o", "--output",
        default="output",
        help="输出目录（默认: output）",
    )
    parser.add_argument(
        "--min-freq",
        type=int,
        default=MIN_FREQUENCY,
        help=f"最低词频（默认: {MIN_FREQUENCY}）",
    )
    parser.add_argument(
        "--max-words",
        type=int,
        default=MAX_WORDS,
        help=f"词云最多显示词数（默认: {MAX_WORDS}）",
    )
    parser.add_argument(
        "--top",
        type=int,
        default=TOP_WORDS_DISPLAY,
        help=f"报告展示 Top N 词（默认: {TOP_WORDS_DISPLAY}）",
    )

    args = parser.parse_args()

    # ── 初始化 ──
    print("=" * 50)
    print(" 微信聊天词云生成工具")
    print("=" * 50)

    # 加载词典
    load_user_dict(USER_DICT_PATH)
    stopwords = load_stopwords(STOPWORDS_PATH)

    # 创建输出目录
    output_dir = Path(args.output)
    output_dir.mkdir(parents=True, exist_ok=True)

    # ── 读取数据 ──
    texts = load_data(args.input)
    if not texts:
        print("[错误] 未提取到任何文本消息，无法生成词云")
        sys.exit(1)

    print(f"\n[信息] 共提取 {len(texts)} 条文本消息")

    # ── 分词 ──
    print("\n[分词] 正在进行中文分词...")
    word_freq = segment_and_count(texts, stopwords, MIN_WORD_LENGTH)

    if not word_freq:
        print("[错误] 分词结果为空，请检查输入数据或停用词表")
        sys.exit(1)

    total_segments = sum(word_freq.values())
    unique_words = len(word_freq)
    print(f"[信息] 分词总数: {total_segments}")
    print(f"[信息] 不同词语: {unique_words}")

    # ── TextRank 短语提取 ──
    print("\n[提取] 正在提取关键词/短语（TextRank）...")
    phrases = extract_key_phrases(texts)
    if phrases:
        print(f"[信息] 提取到 {len(phrases)} 个关键词/短语")

    # ── 合并频率 ──
    word_data = merge_word_frequencies(
        word_freq, phrases,
        min_freq=args.min_freq,
        max_words=args.max_words,
    )

    if not word_data:
        print(f"[错误] 过滤后无词语达到最低词频 {args.min_freq}，请降低 --min-freq")
        sys.exit(1)

    sorted_words = sorted(word_data.items(), key=lambda x: -x[1])
    print(f"\n[词云] 加载 {len(word_data)} 个词语")

    # 打印 top 20
    print("\n  Top 20 高频词:")
    print("  " + "-" * 45)
    for i, (word, count) in enumerate(sorted_words[:20], 1):
        print(f"  {i:2d}. {word:<10s} {count:>5d} 次")
    print("  " + "-" * 45)

    # ── 生成词云 ──
    print("\n[词云] 正在生成词云图...")
    wc_path = generate_wordcloud(word_data, output_dir)

    # ── 生成频率图 ──
    chart_path = generate_frequency_chart(sorted_words, output_dir)

    # ── 统计信息 ──
    stats = {
        "message_count": len(texts),
        "total_segments": total_segments,
        "unique_words": unique_words,
        "top1_word": sorted_words[0][0] if sorted_words else "-",
    }

    # ── 生成 HTML 报告 ──
    print("\n[报告] 正在生成 HTML 报告...")
    report_path = generate_html_report(
        sorted_words, wc_path, chart_path or "", stats, output_dir,
    )

    # ── 完成 ──
    print(f"\n✅ 完成！所有输出文件在: {output_dir.resolve()}")
    print(f"   - 词云图: {Path(wc_path).name}")
    if chart_path:
        print(f"   - 频率图: {Path(chart_path).name}")
    print(f"   - 报告页: {Path(report_path).name}")


if __name__ == "__main__":
    main()
