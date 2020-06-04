using System.Collections.Specialized;
using Anna.Request;
using common;
using common.resources;

namespace server.account
{
    class checkQuestIsDone : RequestHandler
    {
        public override void HandleRequest(RequestContext context, NameValueCollection query)
        {
            DbAccount acc;
            var status = Database.Verify(query["guid"], query["password"], out acc);
            if (status != LoginStatus.OK)
            {
                Write(context, $"<Error>{status.GetInfo()}</Error>");
                return;
            }

            var qst = Program.Resources.Quests[query["questId"].ToInt32()];
            if (qst == null)
            {
                Write(context, "<Error>Invalid questId</Error>");
                return;
            }

            var result = qst.checkQuest(Database, acc).Result;
            switch (result)
            {
                case QuestResult.AlreadyCompletedQuest:
                    Write(context, "<Error>You already completed the quest!</Error>");
                    return;
                case QuestResult.TransactionFailed:
                    Write(context, "<Error>Transaction failed.</Error>");
                    return;
            }
            Write(context, "<Success/>");
        }
    }
}
