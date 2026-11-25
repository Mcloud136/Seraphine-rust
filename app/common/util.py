import json
import os
import re
import winreg
from pathlib import Path

import requests
import base64
import ctypes
from ctypes import wintypes
import psutil
import win32api
import win32gui

from PyQt5.QtCore import QRectF

from app.common.config import cfg, VERSION
from app.common.logger import logger


TAG = "Util"


class Github:
    def __init__(self, user="Zzaphkiel", repositories="Seraphine"):
        self.githubApi = "http://api.github.com"
        self.giteeApi = "http://gitee.com/api"

        self.proxyApi = 'https://ghproxy.net'

        self.user = user
        self.repositories = repositories
        self.sess = requests.session()

    def getReleasesInfo(self):
        url = f"{self.githubApi}/repos/{self.user}/{self.repositories}/releases/latest"

        if cfg.get(cfg.enableGithubProxy):
            proxy = {'https': cfg.get(cfg.githubProxyAddr)}
        else:
            proxy = None

        return self.sess.get(url, proxies=proxy).json()

    def checkUpdate(self):
        """
        检查版本更新
        @return: 有更新 -> info, 无更新 -> None
        """
        info = self.getReleasesInfo()

        ver_info = self.__getVersionInfo()
        info["forbidden"] = ver_info.get("forbidden", False)

        if info.get("tag_name")[1:] != VERSION:
            return info
        return None

    def __getVersionInfo(self):
        url = f'{self.githubApi}/repos/{self.user}/{self.repositories}/contents/document/ver.json'
        if cfg.get(cfg.enableGithubProxy):
            proxy = {'https': cfg.get(cfg.githubProxyAddr)}
        else:
            proxy = None

        res = self.sess.get(url, proxies=proxy).json()

        json_data = json.loads(
            str(base64.b64decode(res['content']), encoding='utf-8'))

        return json_data.get(VERSION, {})

    def getNotice(self):
        url = f'{self.githubApi}/repos/{self.user}/{self.repositories}/contents/document/notice.md'

        if cfg.get(cfg.enableGithubProxy):
            proxy = {'https': cfg.get(cfg.githubProxyAddr)}
        else:
            proxy = None

        res = self.sess.get(url, proxies=proxy).json()

        content = str(base64.b64decode(res['content']), encoding='utf-8')

        return {
            'sha': res['sha'],
            'content': content,
        }


github = Github()


def getLoLPathByRegistry() -> str:
    """
    从注册表获取LOL的安装路径

    ** 只能获取到国服的路径, 外服不支持 **

    无法获取时返回空串
    """
    mainKey = winreg.HKEY_CURRENT_USER
    subKey = "SOFTWARE\Tencent\LOL"
    valueName = "InstallPath"

    try:
        with winreg.OpenKey(mainKey, subKey) as k:
            installPath, _ = winreg.QueryValueEx(k, valueName)
            path = str(Path(f"{installPath}\TCLS").absolute()
                       ).replace("\\", "/")
            return f"{path[:1].upper()}{path[1:]}"
    except FileNotFoundError:
        logger.warning("reg path or val does not exist.", TAG)
    except WindowsError as e:
        logger.warning(f"occurred while reading the registry: {e}", TAG)
    except Exception as e:
        logger.exception("unknown error reading registry", e, TAG)

    return ""


class PROCESSENTRY32W(ctypes.Structure):
    _fields_ = [
        ("dwSize", wintypes.DWORD),
        ("cntUsage", wintypes.DWORD),
        ("th32ProcessID", wintypes.DWORD),
        ("th32DefaultHeapID", ctypes.POINTER(ctypes.c_ulong)),
        ("th32ModuleID", wintypes.DWORD),
        ("cntThreads", wintypes.DWORD),
        ("th32ParentProcessID", wintypes.DWORD),
        ("pcPriClassBase", wintypes.LONG),
        ("dwFlags", wintypes.DWORD),
        ("szExeFile", wintypes.WCHAR * 260),
    ]


class PROCESS_BASIC_INFORMATION(ctypes.Structure):
    _fields_ = [
        ("Reserved1", ctypes.c_void_p),
        ("PebBaseAddress", ctypes.c_void_p),
        ("Reserved2", ctypes.c_void_p * 2),
        ("UniqueProcessId", ctypes.c_void_p),
        ("Reserved3", ctypes.c_void_p),
    ]


def _enumerate_processes():
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateToolhelp32Snapshot.argtypes = [wintypes.DWORD, wintypes.DWORD]
    kernel32.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    kernel32.Process32FirstW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
    kernel32.Process32FirstW.restype = wintypes.BOOL
    kernel32.Process32NextW.argtypes = [wintypes.HANDLE, ctypes.POINTER(PROCESSENTRY32W)]
    kernel32.Process32NextW.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]

    snapshot = kernel32.CreateToolhelp32Snapshot(0x00000002, 0)
    if snapshot == -1:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        entry = PROCESSENTRY32W()
        entry.dwSize = ctypes.sizeof(entry)
        processes = []
        success = kernel32.Process32FirstW(snapshot, ctypes.byref(entry))
        while success:
            processes.append((entry.th32ProcessID, entry.szExeFile))
            success = kernel32.Process32NextW(snapshot, ctypes.byref(entry))
        return processes
    finally:
        kernel32.CloseHandle(snapshot)


def _parse_lcu_command_line(command_line):
    values = {"port": None, "token": None, "server": None}
    for argument in command_line.split():
        argument = argument.strip('"')
        for key, prefix in (
            ("port", "--app-port="),
            ("token", "--remoting-auth-token="),
            ("server", "--rso_platform_id="),
        ):
            if argument.startswith(prefix):
                values[key] = argument[len(prefix):]
    return values["port"], values["token"], values["server"]


def _read_process_command_line(pid):
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    ntdll = ctypes.WinDLL("ntdll")
    kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel32.OpenProcess.restype = wintypes.HANDLE
    kernel32.ReadProcessMemory.argtypes = [
        wintypes.HANDLE,
        ctypes.c_void_p,
        ctypes.c_void_p,
        ctypes.c_size_t,
        ctypes.POINTER(ctypes.c_size_t),
    ]
    kernel32.ReadProcessMemory.restype = wintypes.BOOL
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    ntdll.NtQueryInformationProcess.argtypes = [
        wintypes.HANDLE,
        wintypes.ULONG,
        ctypes.c_void_p,
        wintypes.ULONG,
        ctypes.POINTER(wintypes.ULONG),
    ]
    ntdll.NtQueryInformationProcess.restype = wintypes.LONG

    handle = kernel32.OpenProcess(0x00000400 | 0x00000010, False, pid)
    if not handle:
        raise ctypes.WinError(ctypes.get_last_error())

    def read(address, size):
        buffer = ctypes.create_string_buffer(size)
        read_bytes = ctypes.c_size_t()
        if not kernel32.ReadProcessMemory(
            handle, address, buffer, size, ctypes.byref(read_bytes)
        ) or read_bytes.value != size:
            raise ctypes.WinError(ctypes.get_last_error())
        return buffer.raw

    try:
        basic = PROCESS_BASIC_INFORMATION()
        returned = wintypes.ULONG()
        status = ntdll.NtQueryInformationProcess(
            handle, 0, ctypes.byref(basic), ctypes.sizeof(basic), ctypes.byref(returned)
        )
        if status != 0:
            raise ctypes.WinError(status)
        parameters = ctypes.c_void_p.from_buffer_copy(read(basic.PebBaseAddress + 0x20, 8))
        command_line = read(parameters.value + 0x70, 16)
        length = int.from_bytes(command_line[:2], "little")
        buffer_address = int.from_bytes(command_line[8:16], "little")
        if length == 0 or length > 64 * 1024:
            raise ValueError("invalid process command line length")
        return read(buffer_address, length).decode("utf-16-le")
    finally:
        kernel32.CloseHandle(handle)


def getTasklistPath():
    try:
        _enumerate_processes()
        return "windows-api"
    except OSError:
        return None


def getLolClientPidSlowly():
    return getLolClientPid(None) or -1


def getLolClientPid(path):
    for pid, name in _enumerate_processes():
        if name.lower() == "leagueclientux.exe":
            return pid
    return 0


def getLolClientPids(path):
    return [
        pid
        for pid, name in _enumerate_processes()
        if name.lower() == "leagueclientux.exe"
    ]


def getLolClientPidsSlowly():
    return getLolClientPids(None)


def isLolGameProcessExist(path):
    return any(
        name.lower() == "league of legends.exe"
        for _, name in _enumerate_processes()
    )


def getPortTokenServerByPidViaPsutil(pid):
    port, token, server = None, None, None

    process = psutil.Process(pid)
    cmdline = process.cmdline()

    for cmd in cmdline:

        p = cmd.find("--app-port=")
        if p != -1:
            port = cmd[11:]

        p = cmd.find("--remoting-auth-token=")
        if p != -1:
            token = cmd[22:]

        p = cmd.find("--rso_platform_id=")
        if p != -1:
            server = cmd[18:]

        if port and token and server:
            break

    return port, token, server


def getPortTokenServerByPid(pid):
    try:
        return getPortTokenServerByPidViaPsutil(pid)
    except Exception:
        return _parse_lcu_command_line(_read_process_command_line(pid))


def getFileProperties(fname):
    """
    Read all attributes of the given file and return them as a dictionary.

    returns : {'FixedFileInfo': {'Signature': -17890115, 'StrucVersion': 65536, 'FileVersionMS': 917513, 'FileVersionLS':
    38012988, 'ProductVersionMS': 917513, 'ProductVersionLS': 38012988, 'FileFlagsMask': 23, 'FileFlags': 0,
    'FileOS': 4, 'FileType': 1, 'FileSubtype': 0, 'FileDate': None}, 'StringFileInfo': {'Comments': None,
    'InternalName': 'League of Legends (TM) Client', 'ProductName': 'League of Legends (TM) Client', 'CompanyName':
    'Riot Games, Inc.', 'LegalCopyright': 'Copyright (C) 2009', 'ProductVersion': '14.9.580.2108', 'FileDescription':
    'League of Legends (TM) Client', 'LegalTrademarks': None, 'PrivateBuild': None, 'FileVersion': '14.9.580.2108',
    'OriginalFilename': 'League of Legends.exe', 'SpecialBuild': None}, 'FileVersion': '14.9.580.2108'}

    """

    propNames = ('Comments', 'InternalName', 'ProductName',
                 'CompanyName', 'LegalCopyright', 'ProductVersion',
                 'FileDescription', 'LegalTrademarks', 'PrivateBuild',
                 'FileVersion', 'OriginalFilename', 'SpecialBuild')

    props = {'FixedFileInfo': None,
             'StringFileInfo': None, 'FileVersion': None}

    try:
        fixedInfo = win32api.GetFileVersionInfo(fname, '\\')
        props['FixedFileInfo'] = fixedInfo
        props['FileVersion'] = "%d.%d.%d.%d" % (fixedInfo['FileVersionMS'] / 65536,
                                                fixedInfo['FileVersionMS'] % 65536, fixedInfo['FileVersionLS'] / 65536,
                                                fixedInfo['FileVersionLS'] % 65536)

        # \VarFileInfo\Translation returns list of available (language, codepage)
        # pairs that can be used to retreive string info. We are using only the first pair.
        lang, codepage = win32api.GetFileVersionInfo(
            fname, '\\VarFileInfo\\Translation')[0]

        # any other must be of the form \StringfileInfo\%04X%04X\parm_name, middle
        # two are language/codepage pair returned from above

        strInfo = {}
        for propName in propNames:
            strInfoPath = u'\\StringFileInfo\\%04X%04X\\%s' % (
                lang, codepage, propName)
            strInfo[propName] = win32api.GetFileVersionInfo(fname, strInfoPath)

        props['StringFileInfo'] = strInfo
    except:
        return {}
    else:
        return props


def getLolClientVersion():
    gamePath = cfg.get(cfg.lolFolder)[0]

    assert gamePath  # Must exist, otherwise, there is a logical issue in the call -- By Hpero4

    # Special handling for the Chinese server -- By Hpero4
    gamePath = gamePath.replace("/TCLS", "")

    lolExe = f"{gamePath}/Game/League of Legends.exe"
    # Check if the client is special? Why is the main LOL program missing? -- By Hpero4
    if not os.path.exists(lolExe):
        raise FileNotFoundError(lolExe)

    fileInfo = getFileProperties(lolExe).get("StringFileInfo", {})
    lolVer = fileInfo.get("ProductVersion") or fileInfo.get("FileVersion")

    assert lolVer

    # Shorten to the major version number
    return re.search(r"\d+\.\d+", lolVer).group(0)


def getLolClientWindowPos() -> QRectF:
    # Get the client window handle
    hwnd = win32gui.FindWindow("RCLIENT", "League of Legends")

    # If there is no client, return None directly
    if not hwnd:
        return None

    # Get the client window position
    # struct RECT {
    #     LONG left;
    #     LONG top;
    #     LONG right;
    #     LONG bottom;
    # }
    rect = win32gui.GetWindowRect(hwnd)

    # When the window is minimized, the aspect ratio is not 16:9, return None directly
    if (rect[3] - rect[1]) / (rect[2] - rect[0]) != 0.5625:
        return None

    return QRectF(rect[0], rect[1], rect[2] - rect[0], rect[3] - rect[1])
