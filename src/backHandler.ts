import { useEffect, useRef } from "react";

/** 一次「系统返回」的消费者：返回 true 表示已处理，false 交给下一层 */
export type BackConsumer = () => boolean;

/** 返回处理栈：后注册的先响应（模态框 / 操作菜单优先于页面默认行为） */
const stack: BackConsumer[] = [];

/** 注册一个返回消费者；enabled 为 false 时既不入栈也不出栈 */
export function useBackHandler(consumer: BackConsumer, enabled = true) {
  // 始终调用最新的 consumer，避免因闭包过期而操作到旧 state
  const ref = useRef(consumer);
  ref.current = consumer;
  useEffect(() => {
    if (!enabled) return;
    const entry: BackConsumer = () => ref.current();
    stack.push(entry);
    return () => {
      const i = stack.indexOf(entry);
      if (i >= 0) stack.splice(i, 1);
    };
  }, [enabled]);
}

/** 从栈顶向下寻找第一个消费者；返回本次返回是否被消费 */
export function dispatchBack(): boolean {
  for (let i = stack.length - 1; i >= 0; i -= 1) {
    if (stack[i]()) return true;
  }
  return false;
}
