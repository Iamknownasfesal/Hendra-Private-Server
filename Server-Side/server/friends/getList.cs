﻿using System.Collections.Specialized;
using Anna.Request;

namespace server.friends
{
    class getList : RequestHandler
    {

        public override void HandleRequest(RequestContext context, NameValueCollection query)
        {
            // TODO
            Write(context, "<Friends></Friends>");
        }
    }
}