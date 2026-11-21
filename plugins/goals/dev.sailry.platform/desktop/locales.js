export function messages(locale) {
  return locale === 'zh-CN' ? {
    empty:'暂无目标', evidence:'完成依据', earlier:'历史目标',
    turns:'轮次', time:'用时', tokens:'Token',
    completed:'已完成', cleared:'已结束', replaced:'已替换',
    pause:'暂停', resume:'继续', end:'结束', reopen:'重新打开', retry:'重试',
    paused:'目标已暂停', resumed:'目标已继续', ended:'目标已结束',
    error:'目标操作失败', uncertain:'结果未确认，请重试', readFailed:'无法读取目标',
  } : {
    empty:'No goal', evidence:'Evidence', earlier:'Earlier goals',
    turns:'Turns', time:'Time', tokens:'Tokens',
    completed:'Completed', cleared:'Ended', replaced:'Replaced',
    pause:'Pause', resume:'Resume', end:'End', reopen:'Reopen', retry:'Retry',
    paused:'Goal paused', resumed:'Goal resumed', ended:'Goal ended',
    error:'Goal action failed', uncertain:'Outcome unconfirmed, retry', readFailed:'Could not read the goal',
  };
}
