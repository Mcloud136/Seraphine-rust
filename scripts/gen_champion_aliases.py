import json
import os
import re

from pypinyin import Style, lazy_pinyin

CATALOG = os.path.expandvars(r"%LOCALAPPDATA%\Seraphine\app\catalog.json")
OUTPUT = r"D:\wxmuma\Seraphine-rust\rust\app\src\championAliases.ts"

# English alias (LCU) -> (official CN name, community nicknames)
CURATED = {
    "annie": ("安妮", ["火女"]),
    "olaf": ("奥拉夫", ["狂战士"]),
    "galio": ("加里奥", ["石像鬼"]),
    "twistedfate": ("崔斯特", ["卡牌", "卡牌大师", "tf"]),
    "xinzhao": ("赵信", ["菊花信", "信爷", "德邦"]),
    "urgot": ("厄加特", ["螃蟹"]),
    "leblanc": ("乐芙兰", ["妖姬", "诡术妖姬"]),
    "vladimir": ("弗拉基米尔", ["吸血鬼"]),
    "fiddlesticks": ("费德提克", ["稻草人", "末日"]),
    "kayle": ("凯尔", ["天使", "审判天使"]),
    "masteryi": ("易", ["易大师", "剑圣", "无极剑圣"]),
    "alistar": ("阿利斯塔", ["牛头", "老牛"]),
    "ryze": ("瑞兹", ["光头", "流浪法师", "符文法师"]),
    "sion": ("塞恩", ["亡灵战神", "老司机"]),
    "sivir": ("希维尔", ["轮子妈"]),
    "soraka": ("索拉卡", ["奶妈", "星妈"]),
    "teemo": ("提莫", ["蘑菇", "提百万"]),
    "tristana": ("崔丝塔娜", ["小炮", "炮娘"]),
    "warwick": ("沃里克", ["狼人"]),
    "nunu": ("努努", ["雪人", "雪人骑士"]),
    "missfortune": ("厄运小姐", ["女枪", "好运姐", "mf"]),
    "ashe": ("艾希", ["寒冰", "寒冰射手"]),
    "tryndamere": ("泰达米尔", ["蛮王", "蛮子", "蛮三刀"]),
    "jax": ("贾克斯", ["武器大师", "武器"]),
    "morgana": ("莫甘娜", ["堕落天使"]),
    "zilean": ("基兰", ["时光", "时光守护者"]),
    "singed": ("辛吉德", ["炼金", "炼金术士"]),
    "evelynn": ("伊芙琳", ["寡妇"]),
    "twitch": ("图奇", ["老鼠", "瘟疫之源"]),
    "karthus": ("卡尔萨斯", ["死歌"]),
    "chogath": ("科加斯", ["大虫子", "虫子"]),
    "amumu": ("阿木木", ["木木", "木乃伊"]),
    "rammus": ("拉莫斯", ["龙龟", "乌龟"]),
    "anivia": ("艾尼维亚", ["冰鸟", "凤凰"]),
    "shaco": ("萨科", ["小丑", "恶魔小丑"]),
    "drmundo": ("蒙多", ["蒙多医生", "祖安狂人"]),
    "sona": ("娑娜", ["琴女", "琴瑟仙女"]),
    "kassadin": ("卡萨丁", ["虚空行者"]),
    "irelia": ("艾瑞莉娅", ["刀妹", "女刀"]),
    "janna": ("迦娜", ["风女", "风暴之怒"]),
    "gangplank": ("普朗克", ["船长", "海洋之灾"]),
    "corki": ("库奇", ["飞机"]),
    "karma": ("卡尔玛", ["扇子妈", "天启者"]),
    "taric": ("塔里克", ["宝石", "宝石骑士"]),
    "veigar": ("维迦", ["小法", "邪恶小法师"]),
    "trundle": ("特朗德尔", ["巨魔", "巨魔之王"]),
    "swain": ("斯维因", ["乌鸦", "诺克萨斯统领"]),
    "caitlyn": ("凯特琳", ["女警", "皮城女警"]),
    "blitzcrank": ("布里茨", ["机器人", "蒸汽机器人"]),
    "malphite": ("墨菲特", ["石头人", "石头", "熔岩巨兽"]),
    "katarina": ("卡特琳娜", ["卡特", "不详", "不祥之刃"]),
    "nocturne": ("魔腾", ["梦魇", "永恒梦魇"]),
    "maokai": ("茂凯", ["大树", "扭曲树精"]),
    "renekton": ("雷克顿", ["鳄鱼", "荒漠屠夫"]),
    "jarvaniv": ("嘉文四世", ["皇子", "j4"]),
    "elise": ("伊莉丝", ["蜘蛛", "蜘蛛女皇"]),
    "orianna": ("奥莉安娜", ["发条", "发条魔灵"]),
    "monkeyking": ("孙悟空", ["猴子", "猴哥", "齐天大圣"]),
    "brand": ("布兰德", ["火男", "复仇焰魂"]),
    "leesin": ("李青", ["盲僧", "瞎子", "盲仔"]),
    "vayne": ("薇恩", ["vn", "暗夜猎手"]),
    "rumble": ("兰博", ["机械公敌"]),
    "cassiopeia": ("卡西奥佩娅", ["蛇女", "魔蛇之拥"]),
    "skarner": ("斯卡纳", ["蝎子", "水晶先锋"]),
    "heimerdinger": ("黑默丁格", ["大头", "大发明家"]),
    "nasus": ("内瑟斯", ["狗头", "沙漠死神"]),
    "nidalee": ("奈德丽", ["豹女", "狂野女猎手"]),
    "udyr": ("乌迪尔", ["兽灵行者"]),
    "poppy": ("波比", ["小锤子", "圣锤之毅"]),
    "gragas": ("古拉加斯", ["酒桶"]),
    "pantheon": ("潘森", ["斯巴达", "潘子", "不屈之枪"]),
    "ezreal": ("伊泽瑞尔", ["ez", "探险家"]),
    "mordekaiser": ("莫德凯撒", ["铁男", "金属大师"]),
    "yorick": ("约里克", ["掘墓", "掘墓者"]),
    "akali": ("阿卡丽", ["akl", "离群之刺"]),
    "kennen": ("凯南", ["电耗子", "狂暴之心"]),
    "garen": ("盖伦", ["德玛", "草丛伦", "德玛西亚之力"]),
    "leona": ("蕾欧娜", ["日女", "曙光", "曙光女神"]),
    "malzahar": ("玛尔扎哈", ["蚂蚱", "虚空先知"]),
    "talon": ("泰隆", ["男刀", "刀锋之影"]),
    "riven": ("锐雯", ["瑞文", "锐萌萌", "放逐之刃"]),
    "kogmaw": ("克格莫", ["大嘴", "深渊巨口"]),
    "shen": ("慎", ["暮光之眼"]),
    "lux": ("拉克丝", ["光辉", "光辉女郎"]),
    "xerath": ("泽拉斯", ["远古巫灵"]),
    "shyvana": ("希瓦娜", ["龙女", "龙血武姬"]),
    "ahri": ("阿狸", ["狐狸", "九尾", "九尾妖狐"]),
    "graves": ("格雷福斯", ["男枪", "张三", "法外狂徒"]),
    "fizz": ("菲兹", ["小鱼人", "小鱼", "潮汐海灵"]),
    "volibear": ("沃利贝尔", ["狗熊", "雷霆咆哮"]),
    "rengar": ("雷恩加尔", ["狮子狗", "傲之追猎者"]),
    "varus": ("韦鲁斯", ["惩戒之箭"]),
    "nautilus": ("诺提勒斯", ["泰坦", "深海泰坦"]),
    "viktor": ("维克托", ["三只手", "机械先驱"]),
    "sejuani": ("瑟庄妮", ["猪妹", "北地之怒"]),
    "fiora": ("菲奥娜", ["剑姬", "无双剑姬"]),
    "ziggs": ("吉格斯", ["炸弹人", "爆破鬼才"]),
    "lulu": ("璐璐", ["露露", "仙灵女巫"]),
    "draven": ("德莱文", ["荣耀行刑官"]),
    "hecarim": ("赫卡里姆", ["人马", "战争之影"]),
    "khazix": ("卡兹克", ["螳螂", "虚空掠夺者"]),
    "darius": ("德莱厄斯", ["诺手", "诺克", "诺克萨斯之手"]),
    "jayce": ("杰斯", ["未来守护者"]),
    "lissandra": ("丽桑卓", ["冰女", "冰霜女巫"]),
    "diana": ("黛安娜", ["皎月", "皎月女神"]),
    "quinn": ("奎因", ["鸟人", "德玛西亚之翼"]),
    "syndra": ("辛德拉", ["球女", "暗黑元首"]),
    "aurelionsol": ("奥瑞利安·索尔", ["龙王", "铸星龙王"]),
    "kayn": ("凯隐", ["影流之镰"]),
    "zoe": ("佐伊", ["暮光星灵"]),
    "zyra": ("婕拉", ["荆棘之兴"]),
    "kaisa": ("卡莎", ["虚空之女"]),
    "seraphine": ("萨勒芬妮", ["星籁歌姬"]),
    "gnar": ("纳尔", ["迷失之牙"]),
    "zac": ("扎克", ["生化魔人"]),
    "yasuo": ("亚索", ["快乐风男", "风男", "哈撒ki", "疾风剑豪"]),
    "velkoz": ("维克兹", ["大眼", "虚空之眼"]),
    "taliyah": ("塔莉垭", ["岩雀"]),
    "camille": ("卡蜜尔", ["青钢影", "腿姐"]),
    "akshan": ("阿克尚", ["影哨"]),
    "belveth": ("卑尔维斯", ["虚空女皇"]),
    "braum": ("布隆", ["弗雷尔卓德之心"]),
    "jhin": ("烬", ["瘸子", "戏命师"]),
    "kindred": ("千珏", ["永猎双子"]),
    "zeri": ("泽丽", ["祖安花火"]),
    "jinx": ("金克丝", ["暴走萝莉"]),
    "tahmkench": ("塔姆", ["蛤蟆", "河流之王", "塔姆·肯奇"]),
    "briar": ("贝蕾亚", ["狂厄蔷薇"]),
    "viego": ("佛耶戈", ["破败王", "佛爷", "破败之王"]),
    "senna": ("赛娜", ["涤魂圣枪"]),
    "lucian": ("卢锡安", ["奥巴马", "圣枪游侠"]),
    "zed": ("劫", ["火影", "影流之主"]),
    "kled": ("克烈", ["暴怒骑士"]),
    "ekko": ("艾克", ["时间刺客"]),
    "qiyana": ("奇亚娜", ["元素女皇"]),
    "vi": ("蔚", ["女拳", "皮城执法官"]),
    "aatrox": ("亚托克斯", ["剑魔", "暗裔剑魔"]),
    "nami": ("娜美", ["唤潮鲛姬"]),
    "azir": ("阿兹尔", ["沙皇", "沙漠皇帝"]),
    "yuumi": ("悠米", ["猫", "猫咪", "魔法猫咪"]),
    "samira": ("莎弥拉", ["沙漠玫瑰"]),
    "thresh": ("锤石", ["魂锁典狱长"]),
    "illaoi": ("俄洛伊", ["触手妈", "海兽祭司"]),
    "reksai": ("雷克塞", ["挖掘机", "虚空遁地兽"]),
    "ivern": ("艾翁", ["翠神"]),
    "kalista": ("卡莉丝塔", ["滑板鞋", "复仇之矛"]),
    "bard": ("巴德", ["星界游神"]),
    "rakan": ("洛", ["幻翎"]),
    "xayah": ("霞", ["逆羽"]),
    "ornn": ("奥恩", ["山隐之焰"]),
    "sylas": ("塞拉斯", ["解脱者"]),
    "neeko": ("妮蔻", ["万花通灵"]),
    "aphelios": ("厄斐琉斯", ["月男", "残月之肃"]),
    "rell": ("芮尔", ["镕铁少女"]),
    "pyke": ("派克", ["血港鬼影"]),
    "vex": ("薇古丝", ["愁云使者"]),
    "yone": ("永恩", ["封魔剑魂"]),
    "ambessa": ("安蓓萨", ["铁血狼母"]),
    "mel": ("梅尔", ["灵魂莲华"]),
    "sett": ("瑟提", ["腕豪", "劲夫"]),
    "lillia": ("莉莉娅", ["含羞蓓蕾"]),
    "gwen": ("格温", ["剪刀妹", "灵罗娃娃"]),
    "renata": ("烈娜塔", ["炼金男爵"]),
    "aurora": ("奥罗拉", ["双界灵兔"]),
    "nilah": ("尼菈", ["不羁之悦"]),
    "ksante": ("奎桑提", ["纳祖芒荣耀"]),
    "smolder": ("斯莫德", ["炽炎雏龙"]),
    "milio": ("米利欧", ["明烛"]),
    "hwei": ("彗", ["异画师"]),
    "naafiri": ("纳亚菲利", ["百裂冥犬"]),
}


def normalize(text):
    return re.sub(r"[^0-9a-z\u4e00-\u9fff]", "", text.lower())


def pinyin_forms(text):
    full = normalize("".join(lazy_pinyin(text)))
    initials = normalize("".join(lazy_pinyin(text, style=Style.FIRST_LETTER)))
    return full, initials


def main():
    catalog = json.load(open(CATALOG, encoding="utf-8"))
    champions = catalog["champs"]
    aliases = catalog["champAliases"]
    reverse = {}
    for alias, champion_id in aliases.items():
        key = str(champion_id)
        if key in champions and int(key) < 60000:
            reverse[key] = alias

    result = {}
    for champion_id, title in champions.items():
        cid = int(champion_id)
        if cid <= 0 or cid >= 60000 or not title:
            continue
        alias = reverse.get(champion_id, "")
        words = [title]
        if alias:
            words.append(alias)
        cn_name, nicknames = CURATED.get(alias, ("", []))
        if cn_name:
            words.append(cn_name)
        words.extend(nicknames)
        keywords = set()
        for word in words:
            normalized = normalize(word)
            if normalized:
                keywords.add(normalized)
            full, initials = pinyin_forms(word)
            if full:
                keywords.add(full)
            if initials:
                keywords.add(initials)
        result[cid] = sorted(keywords)

    lines = [
        "// Generated by gen_champion_aliases.py from the LCU catalog plus community nicknames.",
        "// Matching is done case-insensitively with spaces and punctuation stripped.",
        "export const CHAMPION_KEYWORDS: Record<number, string[]> = {",
    ]
    for cid in sorted(result):
        values = ", ".join(json.dumps(word, ensure_ascii=False) for word in result[cid])
        lines.append(f"  {cid}: [{values}],")
    lines.append("};")
    with open(OUTPUT, "w", encoding="utf-8") as handle:
        handle.write("\n".join(lines) + "\n")
    print("champions", len(result), "bytes", os.path.getsize(OUTPUT))


if __name__ == "__main__":
    main()