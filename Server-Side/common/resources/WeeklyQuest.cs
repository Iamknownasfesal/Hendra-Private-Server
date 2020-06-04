using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Linq;
using System.Threading.Tasks;
using System.Xml.Linq;
using System.Xml.XPath;
using log4net;
using Newtonsoft.Json;
using StackExchange.Redis;

namespace common.resources
{
    public struct Reward
    {
        public ushort Item { get; set; }
        public int Count { get; set; }
    }
    public struct Quest
    {
        public string QuestType { get; set; }
        public ushort QuestObject { get; set; }
        public int QuestCount { get; set; }
    }
    public struct RewardContents
    {
        public Reward[] Items { get; set; }
    }
    public enum QuestResult
    {
        Success,
        AlreadyCompletedQuest,
        TransactionFailed,
        Error
    }
    public class Quests
    {
        private static readonly ILog Log = LogManager.GetLogger(typeof(Quest));

        public int Id { get; private set; }
        public string HeadTitle { get; private set; }
        public Quest QuestLul { get; private set; }
        public string Description { get; private set; }
        public int Weight { get; private set; }
        public int questAlreadyDone { get; private set; }
        public DateTime EndDate { get; private set; }
        public RewardContents Contents { get; private set; }


        private string _key;

        public Quests(XElement quests)
        {
            Id = quests.Attribute("id").Value.ToInt32();
            HeadTitle = quests.Element("HeadTitle").Value;
            Description = quests.Element("Description").Value;
            questAlreadyDone = quests.Element("questAlreadyDone").Value.ToInt32();
            Contents = JsonConvert.DeserializeObject<RewardContents>(quests.Element("Contents").Value);
            Weight = quests.Element("Weight").Value.ToInt32();            
            QuestLul = JsonConvert.DeserializeObject<Quest>(quests.Element("QuestData").Value);
            EndDate = DateTime.ParseExact(
                quests.Element("EndDate").Value,
                "MM/dd/yyyy HH:mm:ss 'GMT'K",
                CultureInfo.InvariantCulture);

            _key = $"quests.{Id}";
        }

            public async Task<QuestResult> checkQuest(Database db, DbAccount acc)
            {
                var tran = db.Conn.CreateTransaction();

                // handle if questAlreadyDone
                ConditionResult questAlreadyDoneResult = null;

                if (questAlreadyDone > -1)
                    questAlreadyDoneResult = tran.AddCondition(Condition.HashNotEqual(_key, "maxQuestDone", questAlreadyDone));
                await tran.HashIncrementAsync(_key, acc.AccountId);

                //save items in to a list
                var items = new List<ushort>();
                foreach (var rewards in Contents.Items ?? new Reward[0])
                    items.AddRange(Enumerable.Repeat(rewards.Item, rewards.Count));
                db.AddGifts(acc, items, tran);

            string QuestType = QuestLul.QuestType;
            int QuestCount = QuestLul.QuestCount;
            ushort QuestObject = QuestLul.QuestObject;

            var t1 = tran.ExecuteAsync();

                var t2 = t1.ContinueWith(t =>
                {
                    var success = !t.IsCanceled && t.Result;
                    if (!success)
                    {
                        if (questAlreadyDoneResult != null && !questAlreadyDoneResult.WasSatisfied)
                            return QuestResult.AlreadyCompletedQuest;
                        return QuestResult.TransactionFailed;
                    }

                    acc.FlushAsync();
                    return QuestResult.Success;
                });

                // await tasks
                try
                {
                    await Task.WhenAll(t1, t2);
                }
                catch (Exception e)
                {
                    Log.Error(e);
                    return QuestResult.Error;
                }

                return t2.Result;
            }
        }

        public class WeeklyQuest
        {
            private readonly Dictionary<int, Quests> _weeklyquest;

            public WeeklyQuest()
            {
                _weeklyquest = new Dictionary<int, Quests>();
            }

            public void Load(string path)
            {
                var data = File.ReadAllText(path);
                var root = XElement.Parse(data);
                foreach (var elem in root.XPathSelectElements("//Quests"))
                {
                    var qst = new Quests(elem);
                    _weeklyquest.Add(qst.Id, qst);
                }
            }

            public Quests this[int index]
            {
                get
                {
                    if (!_weeklyquest.ContainsKey(index))
                        return null;

                    var qst = _weeklyquest[index];
                    return qst.EndDate < DateTime.UtcNow ? null : qst;
                }
            }

        }
    }
