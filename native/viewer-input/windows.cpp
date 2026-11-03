#define WIN32_LEAN_AND_MEAN
#include <windows.h>
#include <atomic>
#include <cstdio>
#include <cstdlib>
#include <cwchar>
#include <thread>
static HWND window;
static WCHAR title[1024];
static HHOOK hook;
static bool held[256];
static std::atomic<ULONGLONG> pulse;
static bool focused() {
    RECT rect;POINT p;WCHAR current[1024];GetWindowTextW(window,current,1024);
    return GetForegroundWindow()==window&&GetWindowRect(window,&rect)&&GetCursorPos(&p)&&PtInRect(&rect,p)&&!wcscmp(title,current);
}
static unsigned keycode(DWORD code,bool extended) {
    if(!extended){if(code<=0x53)return code;if(code==0x56)return 86;if(code==0x57)return 87;if(code==0x58)return 88;if(code>=0x64&&code<=0x6f)return code-0x64+183;}
    switch(code){case 0x1c:return 96;case 0x1d:return 97;case 0x35:return 98;case 0x37:return 99;case 0x38:return 100;case 0x45:return 69;case 0x47:return 102;case 0x48:return 103;case 0x49:return 104;case 0x4b:return 105;case 0x4d:return 106;case 0x4f:return 107;case 0x50:return 108;case 0x51:return 109;case 0x52:return 110;case 0x53:return 111;case 0x5b:return 125;case 0x5c:return 126;case 0x5d:return 127;case 0x20:return 113;case 0x2e:return 114;case 0x30:return 115;}return 0;
}
static LRESULT CALLBACK keyboard(int code,WPARAM kind,LPARAM data){
    if(code!=HC_ACTION)return CallNextHookEx(hook,code,kind,data);
    auto *k=reinterpret_cast<KBDLLHOOKSTRUCT*>(data);
    if((k->flags&LLKHF_INJECTED)||!focused())return CallNextHookEx(hook,code,kind,data);
    unsigned key=k->vkCode==VK_PAUSE?119:keycode(k->scanCode,k->flags&LLKHF_EXTENDED);
    if(!key||key>=256)return 1;bool down=kind==WM_KEYDOWN||kind==WM_SYSKEYDOWN;held[key]=down;
    if(down&&key==1&&(held[29]||held[97])&&(held[56]||held[100])&&(held[42]||held[54])){puts("{\"type\":\"released\",\"reason\":\"escape\"}");PostQuitMessage(0);return 1;}
    printf("{\"type\":\"key\",\"code\":%u,\"pressed\":%s}\n",key,down?"true":"false");return 1;
}
int main(int argc,char **argv){
    if(argc!=2)return 2;setvbuf(stdout,nullptr,_IONBF,0);SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    window=GetForegroundWindow();GetWindowTextW(window,title,1024);
    WCHAR expected[1024];MultiByteToWideChar(CP_UTF8,0,argv[1],-1,expected,1024);
    if(!window||(!wcsstr(title,expected)&&!wcsstr(title,L"LightOS")&&!wcsstr(title,L"远程")&&!wcsstr(title,L"Remote"))||!focused())return 3;
    pulse=GetTickCount64();
    std::thread([]{char b;DWORD n;while(ReadFile(GetStdHandle(STD_INPUT_HANDLE),&b,1,&n,nullptr)&&n)pulse=GetTickCount64();ExitProcess(0);}).detach();
    std::thread([]{for(;;){Sleep(25);if(GetTickCount64()-pulse.load()>1500||!focused())ExitProcess(0);}}).detach();
    hook=SetWindowsHookExW(WH_KEYBOARD_LL,keyboard,GetModuleHandleW(nullptr),0);if(!hook)return 4;
    puts("{\"type\":\"ready\",\"backend\":\"windows\"}");
    const struct{int vk;unsigned code;} mods[]={{VK_LCONTROL,29},{VK_RCONTROL,97},{VK_LSHIFT,42},{VK_RSHIFT,54},{VK_LMENU,56},{VK_RMENU,100},{VK_LWIN,125},{VK_RWIN,126}};
    for(auto m:mods)if(GetAsyncKeyState(m.vk)&0x8000){held[m.code]=true;printf("{\"type\":\"key\",\"code\":%u,\"pressed\":true}\n",m.code);}
    MSG msg;while(GetMessageW(&msg,nullptr,0,0)>0){TranslateMessage(&msg);DispatchMessageW(&msg);}UnhookWindowsHookEx(hook);return 0;
}
