/* OpenGL 1.x test (opengl32/glu32/wgl, no extensions): a shaded, depth-tested cube in a window, presented with SwapBuffers. */
#include <windows.h>
#include <GL/gl.h>
#include <GL/glu.h>
static HWND win; static HDC dc; static HGLRC rc; static int frames;
static void quad(float r, float g, float b, const float *v)
{
    glColor3f(r, g, b);
    for (int i = 0; i < 4; i++) { if (i == 2) glColor3f(r * 0.5f, g * 0.5f, b * 0.5f); glVertex3f(v[i * 3], v[i * 3 + 1], v[i * 3 + 2]); }
}
static void draw(void)
{
    glViewport(0, 0, 400, 300);
    glClearColor(0.08f, 0.10f, 0.22f, 1.0f);
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    glEnable(GL_DEPTH_TEST);
    glMatrixMode(GL_PROJECTION); glLoadIdentity(); gluPerspective(45.0, 4.0 / 3.0, 1.0, 20.0);
    glMatrixMode(GL_MODELVIEW); glLoadIdentity();
    glTranslatef(0.0f, 0.0f, -5.0f); glRotatef(28.0f, 1.0f, 0.0f, 0.0f); glRotatef(38.0f, 0.0f, 1.0f, 0.0f);
    static const float f[6][12] = {
        { -1,-1, 1,  1,-1, 1,  1, 1, 1, -1, 1, 1 }, { -1,-1,-1, -1, 1,-1,  1, 1,-1,  1,-1,-1 },
        { -1, 1,-1, -1, 1, 1,  1, 1, 1,  1, 1,-1 }, { -1,-1,-1,  1,-1,-1,  1,-1, 1, -1,-1, 1 },
        {  1,-1,-1,  1, 1,-1,  1, 1, 1,  1,-1, 1 }, { -1,-1,-1, -1,-1, 1, -1, 1, 1, -1, 1,-1 } };
    static const float col[6][3] = { {1,0.2f,0.2f}, {0.2f,1,0.2f}, {0.3f,0.4f,1}, {1,1,0.2f}, {1,0.3f,1}, {0.2f,1,1} };
    glBegin(GL_QUADS); for (int i = 0; i < 6; i++) quad(col[i][0], col[i][1], col[i][2], f[i]); glEnd();
    glDisable(GL_DEPTH_TEST);
    glMatrixMode(GL_PROJECTION); glLoadIdentity(); glOrtho(0, 400, 300, 0, -1, 1);
    glMatrixMode(GL_MODELVIEW); glLoadIdentity();
    glBegin(GL_TRIANGLES); glColor3f(1, 0, 0); glVertex2f(12, 80); glColor3f(0, 1, 0); glVertex2f(72, 80); glColor3f(0, 0, 1); glVertex2f(42, 20); glEnd();
    SwapBuffers(dc); frames++;
}
static LRESULT CALLBACK proc(HWND h, UINT m, WPARAM w, LPARAM l)
{
    if (m == WM_PAINT) { PAINTSTRUCT ps; BeginPaint(h, &ps); EndPaint(h, &ps); draw(); return 0; }
    if (m == WM_DESTROY) { PostQuitMessage(0); return 0; }
    return DefWindowProcW(h, m, w, l);
}
void start(void)
{
    WNDCLASSW c = {0}; c.lpfnWndProc = proc; c.lpszClassName = L"LoGL"; RegisterClassW(&c);
    win = CreateWindowExW(0, L"LoGL", L"OpenGL on LoOS (software)", WS_OVERLAPPEDWINDOW | WS_VISIBLE, 200, 120, 408, 330, 0, 0, 0, 0);
    dc = GetDC(win);
    PIXELFORMATDESCRIPTOR pfd = {0}; pfd.nSize = sizeof pfd; pfd.nVersion = 1; pfd.dwFlags = PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER; pfd.cColorBits = 32; pfd.cDepthBits = 24;
    SetPixelFormat(dc, ChoosePixelFormat(dc, &pfd), &pfd);
    rc = wglCreateContext(dc); wglMakeCurrent(dc, rc);
    ShowWindow(win, 1); draw();
    MSG m; while (GetMessageW(&m, 0, 0, 0) > 0) { TranslateMessage(&m); DispatchMessageW(&m); }
    ExitProcess(0);
}
