import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ApprovalDecision, ApprovalRequest, CreateTaskRequest } from "@symphony/shared-types";
import { mobileKeys } from "../liveData.js";
import { useSession } from "../session.js";

export function useRuntimeState() {
  const { client, pollInterval } = useSession();
  return useQuery({ queryKey: mobileKeys.state, queryFn: ({ signal }) => client.getState(signal), refetchInterval: pollInterval });
}

export function useTasks() {
  const { client, pollInterval } = useSession();
  return useQuery({ queryKey: mobileKeys.tasks, queryFn: ({ signal }) => client.listTasks({}, signal), refetchInterval: pollInterval * 2 });
}

export function usePendingApprovals() {
  const { client, pollInterval } = useSession();
  return useQuery({ queryKey: mobileKeys.approvals, queryFn: ({ signal }) => client.listApprovals(true, signal), refetchInterval: pollInterval });
}

export function useTaskEvents(taskId: string, enabled: boolean) {
  const { client } = useSession();
  return useQuery({ queryKey: mobileKeys.taskEvents(taskId), queryFn: ({ signal }) => client.listTaskEvents(taskId, 20, signal), enabled });
}

export function useDecideApproval() {
  const { client } = useSession();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ approval, decision }: { approval: ApprovalRequest; decision: ApprovalDecision }) => client.decideApproval(approval.id, decision),
    onSettled: () => queryClient.invalidateQueries({ queryKey: mobileKeys.approvals })
  });
}

export function useCancelTask() {
  const { client } = useSession();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (taskId: string) => client.cancelTask(taskId),
    onSettled: () => queryClient.invalidateQueries({ queryKey: mobileKeys.all })
  });
}

export function useProjects() {
  const { client } = useSession();
  return useQuery({ queryKey: mobileKeys.projects, queryFn: ({ signal }) => client.listProjects(signal), staleTime: 60_000 });
}

export function useCreateTask() {
  const { client } = useSession();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateTaskRequest) => client.createTask(input),
    onSettled: () => queryClient.invalidateQueries({ queryKey: mobileKeys.all })
  });
}

export function useDispatchTask() {
  const { client } = useSession();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (taskId: string) => client.dispatchTask(taskId),
    onSettled: () => queryClient.invalidateQueries({ queryKey: mobileKeys.all })
  });
}
