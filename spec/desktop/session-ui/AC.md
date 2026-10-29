# 场景 1：通过同一按钮连续展开和收起工具栏
ID: SC-TOGGLES-ONE-CONTINUOUS-TOOLBAR
Profile: draft
Gate: required
Given 用户打开远程桌面并将悬浮按钮停靠在任意屏幕边缘
When 用户点击悬浮按钮展开工具栏，并在动画途中再次点击收起
Then 同一个外壳从按钮逐渐延伸并从当前状态反向收拢，背景回到透明，图标反映当前展开状态
And 全程只有一个展开和收起按钮，状态小点位于展开工具栏的电脑图标左侧，收起后不显示小点

# 场景 2：切换设置内容时窗口保持稳定
ID: SC-KEEPS-DIALOG-GEOMETRY-ACROSS-TABS
Profile: draft
Gate: required
Given 用户打开设置窗口，浏览器视口保持不变
When 用户依次切换显示、外观、MCP、RDP 和关于标签，并滚动超出窗口的内容
Then 窗口外框高度与位置不变，内容区域独立滚动，标签与右上角关闭按钮保持可见，Tab 栏与内容之间保留独立间距
And 设置窗口不显示“设置”标题文本，没有底部栏和底部完成按钮，关闭再打开仍保留已经保存的偏好

# 场景 3：自动刷新保留正在编辑的内容
ID: SC-REFRESHES-STATUS-WITHOUT-LOSING-EDITS
Profile: draft
Gate: required
Given 用户打开远程桌面设置并正在编辑 MCP 令牌名称或尚未提交的 RDP 端口
When 用户切换到其他页面后返回，或保持当前设置页可见超过一个刷新周期
Then 接入状态自动更新，正在编辑的内容保持不变，当前桌面连接不因刷新而切换
And 桌面来源和显示器列表自动更新，相关界面没有手动刷新按钮

# 场景 4：停靠与拖动时方向反馈及时一致
ID: SC-DOCKS-HORIZONTALLY-WITH-LIVE-DIRECTION-FEEDBACK
Profile: draft
Gate: required
Given 用户在桌面浏览器中打开远程桌面
When 用户按住悬浮按钮并依次拖向左侧、右侧、顶部和底部，途中不松手
Then 达到拖动阈值后光标变为拖拽样式，箭头在拖动途中预览停靠方向，顶部向下、底部向上，松手后收起状态保持该方向
When 用户在靠近某个边缘的页面内部松手，或拖动中丢失浏览器指针捕获
Then 按钮从最后显示的位置吸附到最近的边缘并保留沿边缘的位置，无需移动到边缘才生效
When 用户在顶部或底部的偏左和偏右位置展开工具栏
Then 工具栏始终横向展开，偏左时向右展开，偏右时向左展开，箭头从上下方向平滑旋转为对应的左右收起方向
And 静止悬停和点击使用可点击光标，收起按钮悬停时背景保持透明；展开工具栏与菜单通过灰阶变化区分悬停、选中和按压，邻近选项的背景之间保留间隔

# 场景 5：在页面内选择并保存选项
ID: SC-SELECTS-OPTIONS-IN-PAGE
Profile: draft
Gate: required
Given 用户打开远程桌面设置或键盘组合键窗口
When 用户打开语言、主题、视频编码或主键下拉框，并使用鼠标或方向键与回车选择
Then 页面内的自绘列表显示当前选择和禁用状态，选择应用到对应功能或偏好，不打开系统选项窗口
And 按 Esc 先关闭选项列表并返回触发控件，不关闭外层设置窗口

# 场景 6：通过自绘滚动条访问超出区域的内容
ID: SC-SCROLLS-CUSTOM-TRACKS-WITHOUT-DUPLICATE-BARS
Profile: draft
Gate: required
Given 设置内容、主键选项列表或文字框内容超过可见区域
When 用户滚动内容、靠近滚动区域边缘或在桌面端拖动滑块
Then 页面绘制的圆角滑块显示并同步内容位置，用户能够访问末尾内容，浏览器滚动条不会同时显示
And 横向工具栏和原始大小桌面保留横向滚动，列表选择和表单输入不受滑块操作影响
