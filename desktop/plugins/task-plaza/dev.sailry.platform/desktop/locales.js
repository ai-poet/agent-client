// Accepted workbench copy from Sailry ef5e82ea.
const en = {
  activity_board_waiting:'Awaiting approval',activity_board_running:'Running',activity_board_empty:'No activity',
  activity_completed:'Completed',activity_failed:'Failed',activity_idle:'Idle',activity_no_project:'Unassigned',
  sessions_unassigned:'Unassigned',chat_new:'New conversation',turn_queued:'Queued',chat_stopping:'Stopping',
  activity_approval:'Awaiting approval',activity_input:'Awaiting reply',activity_running:'Running',
  chat_interrupted:'Interrupted',turn_cancelled:'Stopped',terminal:'Terminal',live_disconnected:'Disconnected',
  activity_read_failed:'Could not load activity',retry:'Retry',
  approval_plaza:'Approvals',no_approvals:'No pending approvals',open_conversation:'Open conversation',
  room_running:'Running',room_approvals:'Approval',room_completed:'Done',room_idle:'Idle',
  approval_count:count => `Approvals ${count}`,floor:(current,total) => `Floor ${current} / ${total}`,
  previous:'Previous',next:'Next',
  empty_room:'Available',close:'Close',tasks:'Tasks',no_tasks:'No tasks',task_count:count => `Tasks ${count}`,
};
const zh = {
  activity_board_waiting:'待审批',activity_board_running:'运行中',activity_board_empty:'暂无活动',
  activity_completed:'已完成',activity_failed:'失败',activity_idle:'空闲',activity_no_project:'未归类',
  sessions_unassigned:'未归类',chat_new:'新建会话',turn_queued:'等待处理',chat_stopping:'正在停止',
  activity_approval:'待批准',activity_input:'待回复',activity_running:'进行中',
  chat_interrupted:'已中断',turn_cancelled:'已停止',terminal:'终端',live_disconnected:'已断开',
  activity_read_failed:'无法加载活动',retry:'重试',
  approval_plaza:'审批广场',no_approvals:'暂无待审批',open_conversation:'打开会话',
  room_running:'运行',room_approvals:'审批',room_completed:'完成',room_idle:'空闲',
  approval_count:count => `待审批 ${count}`,floor:(current,total) => `楼层 ${current} / ${total}`,
  previous:'上一页',next:'下一页',
  empty_room:'空房间',close:'关闭',tasks:'任务',no_tasks:'暂无任务',task_count:count => `任务 ${count}`,
};
export const messages = locale => locale === 'zh-CN' ? zh : en;
